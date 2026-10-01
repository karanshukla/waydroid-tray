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

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "waydroid-tray-apps-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            fs::write(dir.join(name), text).unwrap();
        }
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let names = list_apps(dir).into_iter().map(|app| app.name).collect();
        fs::remove_dir_all(dir).unwrap();
        names
    }

    #[test]
    fn lists_visible_apps() {
        let dir = dir_with(&[("waydroid.a.b.desktop", "[Desktop Entry]\nName=Ab\n")]);
        assert_eq!(names(&dir), ["Ab"]);
    }

    #[test]
    fn hides_apps_marked_no_display() {
        let dir = dir_with(&[
            (
                "waydroid.a.desktop",
                "[Desktop Entry]\nName=A\nNoDisplay=true\n",
            ),
            (
                "waydroid.b.desktop",
                "[Desktop Entry]\nName=B\nNoDisplay=false\n",
            ),
        ]);
        assert_eq!(names(&dir), ["B"]);
    }

    #[test]
    fn sorts_apps_by_name_ignoring_case() {
        let dir = dir_with(&[
            ("waydroid.z.desktop", "[Desktop Entry]\nName=zebra\n"),
            ("waydroid.m.desktop", "[Desktop Entry]\nName=Mango\n"),
            ("waydroid.a.desktop", "[Desktop Entry]\nName=Apple\n"),
        ]);
        assert_eq!(names(&dir), ["Apple", "Mango", "zebra"]);
    }

    #[test]
    fn reads_fields_from_the_desktop_entry_group() {
        let text = "[Desktop Entry]\nName=Real\n[Desktop Action x]\nName=Action\n";
        assert_eq!(desktop_entry(text)["Name"], "Real");
    }

    #[test]
    fn ignores_fields_outside_the_desktop_entry_group() {
        let fields = desktop_entry("[Desktop Action x]\nName=Action\n");
        assert!(fields.is_empty());
    }
}
