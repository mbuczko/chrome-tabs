use chrome_tabs::{Bookmark, BookmarkFile, BookmarkNode, Browser};

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

pub fn load_bookmarks(browser: Browser) -> Result<Vec<Bookmark>, String> {
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
