use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Response, Server, StatusCode};

use chrome_tabs::*;

const CACHE_TTL: Duration = Duration::from_secs(10);

// ── Browser ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Browser {
    Chrome,
    Brave,
}

impl Browser {
    fn app_name(self) -> &'static str {
        match self {
            Browser::Chrome => "Google Chrome",
            Browser::Brave => "Brave Browser",
        }
    }

    fn bookmarks_path(self) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let rel = match self {
            Browser::Chrome => "Library/Application Support/Google/Chrome/Default/Bookmarks",
            Browser::Brave => {
                "Library/Application Support/BraveSoftware/Brave-Browser/Default/Bookmarks"
            }
        };
        PathBuf::from(home).join(rel)
    }
}

// ── Tab cache ────────────────────────────────────────────────────────────────

struct TabCache {
    tabs: Vec<Tab>,
    fetched_at: Instant,
}

fn jxa_get_tabs(browser: Browser) -> String {
    format!(
        r#"var app = Application("{app}");
var result = [];
var windows = app.windows();
for (var wi = 0; wi < windows.length; wi++) {{
    var tabs = windows[wi].tabs();
    for (var ti = 0; ti < tabs.length; ti++) {{
        result.push({{
            title: tabs[ti].title(),
            url: tabs[ti].url(),
            window_id: windows[wi].id(),
            window_index: wi,
            tab_index: ti
        }});
    }}
}}
JSON.stringify(result);"#,
        app = browser.app_name()
    )
}

fn fetch_tabs(browser: Browser, buf: &mut Vec<Tab>) -> Result<(), String> {
    let script = jxa_get_tabs(browser);
    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &script])
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("osascript error: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let fresh: Vec<Tab> = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("failed to parse JSON: {e}\nraw: {stdout}"))?;

    buf.clear();
    buf.extend(fresh);
    Ok(())
}

fn start_tab_refresher(browser: Browser, cache: Arc<RwLock<TabCache>>) {
    thread::spawn(move || {
        let mut buf = std::mem::take(&mut cache.write().unwrap().tabs);
        loop {
            match fetch_tabs(browser, &mut buf) {
                Ok(()) => {
                    let mut guard = cache.write().unwrap();
                    std::mem::swap(&mut guard.tabs, &mut buf);
                    guard.fetched_at = Instant::now();
                }
                Err(e) => eprintln!("tab cache refresh failed: {e}"),
            }
            thread::sleep(CACHE_TTL);
        }
    });
}

fn get_cached_tabs(cache: &Arc<RwLock<TabCache>>) -> Vec<Tab> {
    cache.read().unwrap().tabs.clone()
}

// ── Bookmarks ────────────────────────────────────────────────────────────────

fn flatten_node(node: &BookmarkNode, folder: &str, out: &mut Vec<Bookmark>) {
    match node.kind.as_str() {
        "url" => {
            if let Some(url) = &node.url {
                out.push(Bookmark {
                    title: node.name.clone(),
                    url: url.clone(),
                    folder: folder.to_string(),
                });
            }
        }
        "folder" => {
            // Build the folder path breadcrumb: "Parent > Child"
            let child_folder = if folder.is_empty() {
                node.name.clone()
            } else {
                format!("{folder} > {}", node.name)
            };
            for child in &node.children {
                flatten_node(child, &child_folder, out);
            }
        }
        _ => {}
    }
}

fn load_bookmarks(browser: Browser) -> Result<Vec<Bookmark>, String> {
    let path = browser.bookmarks_path();
    let data = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read bookmarks file {}: {e}", path.display()))?;

    let file: BookmarkFile =
        serde_json::from_str(&data).map_err(|e| format!("failed to parse bookmarks JSON: {e}"))?;

    let mut bookmarks = Vec::new();
    flatten_node(&file.roots.bookmark_bar, "Bookmarks bar", &mut bookmarks);
    flatten_node(&file.roots.other, "Other bookmarks", &mut bookmarks);
    flatten_node(&file.roots.synced, "Mobile bookmarks", &mut bookmarks);

    eprintln!("loaded {} bookmarks", bookmarks.len());
    Ok(bookmarks)
}

// ── Focus tab ────────────────────────────────────────────────────────────────

fn jxa_focus_tab(browser: Browser, window_id: &str, tab_index: usize) -> String {
    format!(
        r#"var app = Application("{app}");
var windows = app.windows();
var win = null;
for (var i = 0; i < windows.length; i++) {{
    if (windows[i].id() === "{wid}") {{ win = windows[i]; break; }}
}}
if (!win) throw new Error("window " + "{wid}" + " not found");
win.activeTabIndex = {ti} + 1;
app.activate();
win.index = 1;
"#,
        app = browser.app_name(),
        wid = window_id,
        ti = tab_index,
    )
}

fn focus_tab(browser: Browser, window_id: &str, tab_index: usize) -> Result<(), String> {
    let script = jxa_focus_tab(browser, window_id, tab_index);
    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &script])
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("osascript error: {stderr}"));
    }
    Ok(())
}

// ── HTTP helpers ─────────────────────────────────────────────────────────────

fn json_header() -> Header {
    Header::from_bytes("Content-Type", "application/json").unwrap()
}

fn respond_json<T: serde::Serialize>(request: tiny_http::Request, status: u16, body: &T) {
    let json = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
    let response = Response::new(
        StatusCode(status),
        vec![json_header()],
        Cursor::new(json.clone()),
        Some(json.len()),
        None,
    );
    let _ = request.respond(response);
}

fn respond_error(request: tiny_http::Request, status: u16, message: &str) {
    let body = serde_json::json!({ "error": message });
    respond_json(request, status, &body);
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let browser = match std::env::args().nth(1).as_deref() {
        Some("--brave") => Browser::Brave,
        _ => Browser::Chrome,
    };

    let addr = "127.0.0.1:9223";
    let server = Server::http(addr).expect("failed to start server");

    // Load bookmarks once at startup.
    let bookmarks: Rc<Vec<Bookmark>> = Rc::new(match load_bookmarks(browser) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("warning: could not load bookmarks: {e}");
            vec![]
        }
    });

    // Spawn background tab-refresher thread.
    let tab_cache: Arc<RwLock<TabCache>> = Arc::new(RwLock::new(TabCache {
        tabs: Vec::with_capacity(300),
        fetched_at: Instant::now(),
    }));
    start_tab_refresher(browser, Arc::clone(&tab_cache));

    println!("chrome-tabs listening on http://{addr}");
    println!("  GET  /tabs         - list open tabs (cached, refreshed every {CACHE_TTL:?})");
    println!("  GET  /bookmarks    - list all bookmarks (loaded once at startup)");
    println!("  POST /focus        - focus a tab, body: {{\"window_index\":0,\"tab_index\":0}}");

    for request in server.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();

        match (method, url.as_str()) {
            (Method::Get, "/tabs") => {
                respond_json(request, 200, &get_cached_tabs(&tab_cache));
            }

            (Method::Get, "/bookmarks") => {
                respond_json(request, 200, &*bookmarks);
            }

            (Method::Post, "/focus") => {
                let mut body = String::new();
                let mut request = request;
                if let Err(e) = std::io::Read::read_to_string(request.as_reader(), &mut body) {
                    respond_error(request, 400, &format!("failed to read body: {e}"));
                    continue;
                }
                match serde_json::from_str::<FocusRequest>(&body) {
                    Ok(focus) => match focus_tab(browser, &focus.window_id, focus.tab_index) {
                        Ok(()) => respond_json(request, 200, &serde_json::json!({"ok": true})),
                        Err(e) => {
                            eprintln!("error focusing tab: {e}");
                            respond_error(request, 500, &e);
                        }
                    },
                    Err(e) => respond_error(request, 400, &format!("invalid body: {e}")),
                }
            }

            (Method::Get, "/health") => {
                let guard = tab_cache.read().unwrap();
                let age = format!("{:.1}s ago", guard.fetched_at.elapsed().as_secs_f32());
                respond_json(
                    request,
                    200,
                    &serde_json::json!({
                        "ok": true,
                        "tabs_cache": age,
                        "bookmarks_count": bookmarks.len()
                    }),
                )
            }

            _ => respond_error(request, 404, "not found"),
        }
    }
}
