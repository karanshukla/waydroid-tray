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
        .filter_map(|entry| {
            let file = entry.file_name().into_string().ok()?;
            let package = file
                .strip_prefix("waydroid.")?
                .strip_suffix(".desktop")
                .filter(|package| is_package_name(package))?
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
                icon: fields
                    .get("Icon")
                    .map(PathBuf::from)
                    .filter(|icon| icon.file_name().is_some() && icon.parent() == Some(icons))
                    .unwrap_or_default(),
                package,
            })
        })
        .collect();
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

/// The package goes to `waydroid app launch` as an argument, so a name that
/// isn't one, like `-h`, mustn't get there.
fn is_package_name(name: &str) -> bool {
    name.split('.').all(|part| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    })
}

/// The icon's bytes, or none if it isn't a regular file of a sensible size.
/// Doesn't follow a symlink, and won't block opening a FIFO.
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

    const ICONS: &str = "/home/u/.local/share/waydroid/data/icons";

    fn apps(dir: &Path) -> Vec<AppEntry> {
        let apps = list_apps(dir, Path::new(ICONS));
        fs::remove_dir_all(dir).unwrap();
        apps
    }

    fn names(dir: &Path) -> Vec<String> {
        apps(dir).into_iter().map(|app| app.name).collect()
    }

    fn icon(line: &str) -> PathBuf {
        let text = format!("[Desktop Entry]\nName=A\n{line}\n");
        let dir = dir_with(&[("waydroid.a.desktop", &text)]);
        apps(&dir).remove(0).icon
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

    #[test]
    fn skips_files_whose_package_isnt_a_package_name() {
        let entry = "[Desktop Entry]\nName=X\n";
        let dir = dir_with(&[
            ("waydroid.-h.desktop", entry),
            ("waydroid.--help.desktop", entry),
            ("waydroid.a..b.desktop", entry),
            ("waydroid.a b.desktop", entry),
            ("waydroid..desktop", entry),
            (
                "waydroid.com.example_app2.desktop",
                "[Desktop Entry]\nName=Ok\n",
            ),
        ]);
        let packages: Vec<String> = apps(&dir).into_iter().map(|app| app.package).collect();
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
        let dir = dir_with(&[("a.png", "png")]);
        let png = read_icon(&dir.join("a.png"));
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(png, b"png");
    }

    #[test]
    fn wont_read_an_oversized_icon() {
        let dir = dir_with(&[]);
        let path = dir.join("big.png");
        File::create(&path)
            .unwrap()
            .set_len(MAX_ICON_BYTES + 1)
            .unwrap();
        let png = read_icon(&path);
        fs::remove_dir_all(&dir).unwrap();
        assert!(png.is_empty());
    }

    #[test]
    fn wont_read_an_icon_through_a_symlink() {
        let dir = dir_with(&[("a.png", "png")]);
        std::os::unix::fs::symlink(dir.join("a.png"), dir.join("link.png")).unwrap();
        let png = read_icon(&dir.join("link.png"));
        fs::remove_dir_all(&dir).unwrap();
        assert!(png.is_empty());
    }

    #[test]
    fn wont_read_a_device_or_fifo() {
        let dir = dir_with(&[]);
        let fifo = dir.join("fifo.png");
        let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let png = read_icon(&fifo);
        fs::remove_dir_all(&dir).unwrap();
        assert!(png.is_empty());
        assert!(read_icon(Path::new("/dev/zero")).is_empty());
    }
}
