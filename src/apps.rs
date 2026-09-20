//! Android apps, read from the launchers Waydroid generates.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppEntry {
    pub name: String,
    pub package: String,
    pub icon: PathBuf,
}

/// Visible Waydroid apps in `dir`, sorted by name.
pub fn list_apps(dir: &Path) -> Vec<AppEntry> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut apps: Vec<AppEntry> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().into_string().ok()?;
            let package = file
                .strip_prefix("waydroid.")?
                .strip_suffix(".desktop")?
                .to_owned();
            let text = fs::read_to_string(entry.path()).ok()?;
            let fields = desktop_entry(&text);
            if fields
                .get("NoDisplay")
                .is_some_and(|v| v.eq_ignore_ascii_case("true"))
            {
                return None;
            }
            Some(AppEntry {
                name: fields
                    .get("Name")
                    .cloned()
                    .unwrap_or_else(|| package.clone()),
                icon: fields.get("Icon").map(PathBuf::from).unwrap_or_default(),
                package,
            })
        })
        .collect();
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

/// Key/value pairs from the `[Desktop Entry]` group only (not the actions).
fn desktop_entry(text: &str) -> HashMap<String, String> {
    let mut in_entry = false;
    let mut fields = HashMap::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if let (true, Some((key, value))) = (in_entry, line.split_once('=')) {
            fields.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    fields
}
