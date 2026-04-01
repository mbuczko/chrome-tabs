use std::process::Command;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chrome_tabs::{Browser, Tab};

pub const CACHE_TTL: Duration = Duration::from_secs(10);

pub struct TabCache {
    pub tabs: Vec<Tab>,
    pub fetched_at: Instant,
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

pub fn fetch_tabs(browser: Browser, buf: &mut Vec<Tab>) -> Result<(), String> {
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

pub fn start_tab_refresher(browser: Browser, cache: Arc<RwLock<TabCache>>) {
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

pub fn get_cached_tabs(cache: &Arc<RwLock<TabCache>>) -> Vec<Tab> {
    cache.read().unwrap().tabs.clone()
}

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

pub fn focus_tab(browser: Browser, window_id: &str, tab_index: usize) -> Result<(), String> {
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
