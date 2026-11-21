//! Android apps, read from the launchers Waydroid generates.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Waydroid's icons are a few KiB of PNG.
const MAX_ICON_BYTES: u64 = 1024 * 1024;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppEntry {
    pub name: String,
    pub package: String,
    pub icon: PathBuf,
}

/// `icons` is Waydroid's icon directory. An `Icon=` anywhere else is dropped.
pub fn list_apps(dir: &Path, icons: &Path) -> Vec<AppEntry> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut apps: Vec<AppEntry> = entries
        .flatten()
        .filter_map(|entry| read_launcher(&entry.path(), icons))
        .collect();
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

const MAX_LAUNCHER_BYTES: u64 = 64 * 1024;

/// A `waydroid.<package>.desktop` launcher, unless it's hidden.
fn read_launcher(path: &Path, icons: &Path) -> Option<AppEntry> {
    let package = package_of(path.file_name()?.to_str()?)?.to_owned();
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_LAUNCHER_BYTES {
        return None;
    }
    let mut text = String::new();
    file.take(MAX_LAUNCHER_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() as u64 > MAX_LAUNCHER_BYTES {
        return None;
    }
    let fields = desktop_entry(&text);
    let hidden = fields
        .get("NoDisplay")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    if hidden {
        return None;
    }
    Some(AppEntry {
        name: fields
            .get("Name")
            .cloned()
            .unwrap_or_else(|| package.clone()),
        icon: fields
            .get("Icon")
            .map(PathBuf::from)
            .filter(|icon| icon.file_name().is_some() && icon.parent() == Some(icons))
            .unwrap_or_default(),
        package,
    })
}

fn package_of(launcher: &str) -> Option<&str> {
    launcher
        .strip_prefix("waydroid.")?
        .strip_suffix(".desktop")
        .filter(|package| is_package_name(package))
}

/// The package goes to `waydroid app launch` as an argument, so a name that
/// isn't one, like `-h`, mustn't get there (pinned by
/// `tests::skips_files_whose_package_isnt_a_package_name`).
fn is_package_name(name: &str) -> bool {
    name.split('.').all(|part| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    })
}

/// The icon's bytes, or none if it isn't a regular file of a sensible size.
/// Doesn't follow a symlink, and won't block opening a FIFO (pinned by
/// `tests::wont_read_an_icon_through_a_symlink`, `tests::wont_read_a_device_or_fifo`).
pub fn read_icon(path: &Path) -> Vec<u8> {
    let read = || {
        let file = File::options()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .ok()?;
        let meta = file.metadata().ok()?;
        if !meta.is_file() || meta.len() > MAX_ICON_BYTES {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(MAX_ICON_BYTES + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= MAX_ICON_BYTES).then_some(bytes)
    };
    read().unwrap_or_default()
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
    use crate::test_support::Scratch;

    const ICONS: &str = "/home/u/.local/share/waydroid/data/icons";

    fn launchers(name: &str, files: &[(&str, &str)]) -> Vec<AppEntry> {
        let dir = Scratch::new(name);
        for (file, text) in files {
            dir.write(file, text);
        }
        list_apps(&dir, Path::new(ICONS))
    }

    fn names(name: &str, files: &[(&str, &str)]) -> Vec<String> {
        launchers(name, files)
            .into_iter()
            .map(|app| app.name)
            .collect()
    }

    fn icon(line: &str) -> PathBuf {
        let text = format!("[Desktop Entry]\nName=A\n{line}\n");
        launchers("icon", &[("waydroid.a.desktop", &text)])
            .remove(0)
            .icon
    }

    #[test]
    fn lists_visible_apps() {
        let files = [("waydroid.a.b.desktop", "[Desktop Entry]\nName=Ab\n")];
        assert_eq!(names("visible", &files), ["Ab"]);
    }

    #[test]
    fn hides_apps_marked_no_display() {
        let files = [
            (
                "waydroid.a.desktop",
                "[Desktop Entry]\nName=A\nNoDisplay=true\n",
            ),
            (
                "waydroid.b.desktop",
                "[Desktop Entry]\nName=B\nNoDisplay=false\n",
            ),
        ];
        assert_eq!(names("no-display", &files), ["B"]);
    }

    #[test]
    fn sorts_apps_by_name_ignoring_case() {
        let files = [
            ("waydroid.z.desktop", "[Desktop Entry]\nName=zebra\n"),
            ("waydroid.m.desktop", "[Desktop Entry]\nName=Mango\n"),
            ("waydroid.a.desktop", "[Desktop Entry]\nName=Apple\n"),
        ];
        assert_eq!(names("sorted", &files), ["Apple", "Mango", "zebra"]);
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

    #[test]
    fn skips_files_whose_package_isnt_a_package_name() {
        let entry = "[Desktop Entry]\nName=X\n";
        let files = [
            ("waydroid.-h.desktop", entry),
            ("waydroid.--help.desktop", entry),
            ("waydroid.a..b.desktop", entry),
            ("waydroid.a b.desktop", entry),
            ("waydroid..desktop", entry),
            ("waydroid.com.example_app2.desktop", entry),
        ];
        let packages: Vec<String> = launchers("packages", &files)
            .into_iter()
            .map(|app| app.package)
            .collect();
        assert_eq!(packages, ["com.example_app2"]);
    }

    #[test]
    fn keeps_icons_in_waydroids_icon_directory() {
        let path = format!("{ICONS}/com.android.calculator2.png");
        assert_eq!(icon(&format!("Icon={path}")), PathBuf::from(path));
    }

    #[test]
    fn drops_icons_anywhere_else() {
        for line in [
            "Icon=/dev/zero",
            "Icon=/home/u/.local/share/waydroid/data/icons/sub/a.png",
            "Icon=/home/u/.local/share/waydroid/data/icons/..",
            "Icon=/home/u/.local/share/waydroid/data/icons/../a.png",
            "Icon=waydroid",
        ] {
            assert_eq!(icon(line), PathBuf::new(), "{line}");
        }
    }

    #[test]
    fn reads_an_icon_file() {
        let dir = Scratch::new("icon-file");
        assert_eq!(read_icon(&dir.write("a.png", "png")), b"png");
    }

    #[test]
    fn wont_read_an_oversized_icon() {
        let dir = Scratch::new("icon-oversized");
        let path = dir.join("big.png");
        File::create(&path)
            .unwrap()
            .set_len(MAX_ICON_BYTES + 1)
            .unwrap();
        assert!(read_icon(&path).is_empty());
    }

    #[test]
    fn wont_read_an_icon_through_a_symlink() {
        let dir = Scratch::new("icon-symlink");
        let target = dir.write("a.png", "png");
        std::os::unix::fs::symlink(target, dir.join("link.png")).unwrap();
        assert!(read_icon(&dir.join("link.png")).is_empty());
    }

    #[test]
    fn wont_read_a_device_or_fifo() {
        let dir = Scratch::new("icon-fifo");
        let fifo = dir.join("fifo.png");
        let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(read_icon(&fifo).is_empty());
        assert!(read_icon(Path::new("/dev/zero")).is_empty());
    }
}
