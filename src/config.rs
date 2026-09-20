//! Settings kept in `$XDG_CONFIG_HOME/waydroid-tray/config`, one `key=value` per line.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Config {
    pub start_at_login: bool,
    pub hide_when_stopped: bool,
    pub auto_stop_when_idle: bool,
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
                "start_at_login={}\nhide_when_stopped={}\nauto_stop_when_idle={}\n",
                self.start_at_login, self.hide_when_stopped, self.auto_stop_when_idle
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
                "auto_stop_when_idle" => config.auto_stop_when_idle = value,
                _ => {}
            }
        }
        config
    }
}

/// The config, where it lives, and a shared copy of the one flag the main
/// loop also reads. Toggling through here keeps all three in step.
pub struct Settings {
    pub config: Config,
    path: PathBuf,
    auto_stop: Arc<AtomicBool>,
}

impl Settings {
    pub fn new(config: Config, path: PathBuf) -> Self {
        let auto_stop = Arc::new(AtomicBool::new(config.auto_stop_when_idle));
        Self {
            config,
            path,
            auto_stop,
        }
    }

    /// For the main loop, which owns the idle timer.
    pub fn auto_stop(&self) -> Arc<AtomicBool> {
        self.auto_stop.clone()
    }

    /// Flips one flag and writes the file back. A failed write is only worth
    /// a log line: the setting still applies until the tray restarts.
    pub fn toggle(&mut self, field: impl Fn(&mut Config) -> &mut bool) {
        *field(&mut self.config) ^= true;
        self.auto_stop
            .store(self.config.auto_stop_when_idle, Ordering::Relaxed);
        if let Err(err) = self.config.save(&self.path) {
            eprintln!("failed to save {}: {err}", self.path.display());
        }
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
            auto_stop_when_idle: true,
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
                hide_when_stopped: false,
                auto_stop_when_idle: false
            }
        );
    }

    #[test]
    fn toggle_writes_through_and_publishes_auto_stop() {
        let path = std::env::temp_dir().join(format!(
            "waydroid-tray-toggle-{}/config",
            std::process::id()
        ));
        let mut settings = Settings::new(Config::default(), path.clone());
        let auto_stop = settings.auto_stop();
        settings.toggle(|config| &mut config.auto_stop_when_idle);
        assert!(auto_stop.load(Ordering::Relaxed));
        assert_eq!(
            Config::load(&path),
            Config {
                auto_stop_when_idle: true,
                ..Config::default()
            }
        );
        settings.toggle(|config| &mut config.auto_stop_when_idle);
        assert!(!auto_stop.load(Ordering::Relaxed));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
