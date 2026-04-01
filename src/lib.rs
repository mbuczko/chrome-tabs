use std::path::PathBuf;

#[derive(Clone, Copy)]
pub enum Browser {
    Chrome,
    Brave,
}

impl Browser {
    pub fn app_name(self) -> &'static str {
        match self {
            Browser::Chrome => "Google Chrome",
            Browser::Brave => "Brave Browser",
        }
    }

    pub fn bookmarks_path(self) -> PathBuf {
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

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct Tab {
    pub title: String,
    pub url: String,
    pub window_id: String, // Chrome's stable window id
    pub window_index: usize,
    pub tab_index: usize,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
    pub folder: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct FocusRequest {
    pub window_id: String,
    pub tab_index: usize,
}

// Raw Chrome bookmark JSON shapes (only what we need)
#[derive(Debug, serde::Deserialize)]
pub struct BookmarkFile {
    pub roots: BookmarkRoots,
}

#[derive(Debug, serde::Deserialize)]
pub struct BookmarkRoots {
    pub bookmark_bar: BookmarkNode,
    pub other: BookmarkNode,
    pub synced: BookmarkNode,
}

#[derive(Debug, serde::Deserialize)]
pub struct BookmarkNode {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub children: Vec<BookmarkNode>,
}
