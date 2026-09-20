//! Settings kept in `$XDG_CONFIG_HOME/waydroid-tray/config`, one `key=value` per line.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Config {
    pub start_at_login: bool,
    pub hide_when_stopped: bool,
}

impl Config {
    pub fn path(home: &Path) -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|dir| !dir.is_empty())
            .map_or_else(|| home.join(".config"), PathBuf::from)
            .join("waydroid-tray/config")
    }

    /// A missing or unreadable file means defaults.
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn save(self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(
            path,
            format!(
                "start_at_login={}\nhide_when_stopped={}\n",
                self.start_at_login, self.hide_when_stopped
            ),
        )
    }

    /// Unknown keys and malformed lines are ignored.
    fn parse(text: &str) -> Self {
        let mut config = Self::default();
        for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
            let value = value.trim() == "true";
            match key.trim() {
                "start_at_login" => config.start_at_login = value,
                "hide_when_stopped" => config.hide_when_stopped = value,
                _ => {}
            }
        }
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let path =
            std::env::temp_dir().join(format!("waydroid-tray-test-{}/config", std::process::id()));
        let config = Config {
            start_at_login: true,
            hide_when_stopped: false,
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path), config);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_file_is_default() {
        assert_eq!(
            Config::load(Path::new("/nonexistent/waydroid-tray/config")),
            Config::default()
        );
    }

    #[test]
    fn malformed_lines_are_ignored() {
        let config =
            Config::parse("garbage\nstart_at_login = true\n=true\nhide_when_stopped=yes\n");
        assert_eq!(
            config,
            Config {
                start_at_login: true,
                hide_when_stopped: false
            }
        );
    }
}
