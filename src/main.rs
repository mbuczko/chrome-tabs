mod bookmarks;
mod http;
mod tabs;

use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tiny_http::{Method, Server};

use bookmarks::load_bookmarks;
use chrome_tabs::{Bookmark, Browser, FocusRequest};
use http::{respond_error, respond_json};
use tabs::{focus_tab, get_cached_tabs, start_tab_refresher, TabCache, CACHE_TTL};

fn check_auth(request: &tiny_http::Request, token: &str) -> bool {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .and_then(|h| h.value.as_str().strip_prefix("Bearer "))
        == Some(token)
}

fn main() {
    let token = std::env::var("CHROME_TABS_TOKEN").expect("CHROME_TABS_TOKEN environment variable is required");

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
    println!("  All requests require: Authorization: Bearer <CHROME_TABS_TOKEN>");
    println!("  GET  /tabs         - list open tabs (cached, refreshed every {CACHE_TTL:?})");
    println!("  GET  /bookmarks    - list all bookmarks (loaded once at startup)");
    println!("  POST /focus        - focus a tab, body: {{\"window_index\":0,\"tab_index\":0}}");

    for request in server.incoming_requests() {
        if !check_auth(&request, &token) {
            respond_error(request, 401, "unauthorized");
            continue;
        }

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
