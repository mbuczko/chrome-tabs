mod bookmarks;
mod tabs;

use std::io::Cursor;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tiny_http::{Header, Method, Response, Server, StatusCode};

use chrome_tabs::{Browser, Bookmark, FocusRequest};
use bookmarks::load_bookmarks;
use tabs::{focus_tab, get_cached_tabs, start_tab_refresher, TabCache, CACHE_TTL};

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
