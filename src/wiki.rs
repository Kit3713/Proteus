// SPDX-License-Identifier: GPL-3.0-or-later

use include_dir::{Dir, include_dir};

static WIKI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/wiki");

pub fn list_pages() -> Vec<String> {
    let mut names: Vec<String> = WIKI
        .files()
        .filter_map(|f| {
            let path = f.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                return None;
            }
            path.file_stem().and_then(|s| s.to_str()).map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

pub fn get_page(name: &str) -> Option<&'static str> {
    let path = format!("{name}.md");
    WIKI.get_file(&path).and_then(|f| f.contents_utf8())
}
