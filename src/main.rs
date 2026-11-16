//! System tray icon for Waydroid: session status, start/stop, and app launcher.

mod apps;
mod bus;
mod cli;
mod config;
mod install;
mod notify;
mod state;
#[cfg(test)]
mod test_support;
mod tray;

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use ksni::TrayMethods;
use tokio::sync::Notify;
use zbus::Connection;
use zbus::fdo::{DBusProxy, NameOwnerChangedStream};
use zbus::names::BusName;

use apps::list_apps;
use bus::{CONTAINER_NAME, SESSION_NAME, gained_owner, read_state};
use cli::spawn_waydroid;
use config::{Config, Settings};
use state::State;
use tray::{Snapshot, WaydroidTray};

/// Also how Running vs Frozen is noticed: freezing doesn't show up on the bus.
const POLL: Duration = Duration::from_secs(5);
/// Waydroid claims its session name before it asks the container to start,
/// so right after the name appears the container still reports no session.
const STARTING_POLL: Duration = Duration::from_millis(500);
const AFTER_MENU_ACTION: Duration = Duration::from_millis(1500);
/// Waydroid freezes the container when no Android windows are open, so the
/// idle stop only ever fires on a session nothing is using.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Overrides `IDLE_TIMEOUT`, in seconds, for the harness.
const IDLE_TIMEOUT_VAR: &str = "WAYDROID_TRAY_IDLE_SECS";

const HELP: &str = concat!(
    env!("CARGO_PKG_NAME"),
    " ",
    env!("CARGO_PKG_VERSION"),
    "\n",
    env!("CARGO_PKG_DESCRIPTION"),
    "\n\nUsage: waydroid-tray [OPTIONS]\n\n",
    "Options:\n",
    "  --install      Install the icons, systemd user unit, and menu and\n",
    "                 autostart entries, then start the tray\n",
    "  --uninstall    Remove them again\n",
    "  -h, --help     Print this help\n",
    "  -V, --version  Print the version\n",
    "\nWith no options it registers the tray icon and runs until it's quit.",
);

/// Answers the flags before anything else, so they work with a tray already
/// holding the lock, and `--version` and `--help` with no bus to talk to.
/// Unrecognised arguments are ignored.
async fn print_flags_and_exit() {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-V" | "--version" => {
                println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            "--install" => exit_with(install::install(&home()).await),
            "--uninstall" => exit_with(install::uninstall(&home()).await),
            _ => {}
        }
    }
}

fn exit_with(result: std::io::Result<()>) -> ! {
    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
    std::process::exit(0);
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME is set"))
}

fn idle_timeout() -> Duration {
    std::env::var(IDLE_TIMEOUT_VAR)
        .ok()
        .and_then(|secs| secs.parse().ok())
        .map_or(IDLE_TIMEOUT, Duration::from_secs)
}

/// Time since boot, counting time the machine spent suspended. `Instant` and
/// tokio's timers run on CLOCK_MONOTONIC, which stops across s2idle, so a
/// laptop that sleeps overnight never accumulates idle time.
fn since_boot() -> Duration {
    fn without_proc() -> Duration {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed()
    }
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse::<f64>().ok())
        .map_or_else(without_proc, Duration::from_secs_f64)
}

/// How much longer a session frozen at `frozen_since` has before the idle stop
/// fires, or `None` if it isn't frozen. Saturates at zero (pinned by
/// `tests::a_session_frozen_across_a_suspend_is_due_on_resume`,
/// `tests::a_clock_that_jumps_backwards_does_not_panic`).
fn idle_left(frozen_since: Option<Duration>, now: Duration, timeout: Duration) -> Option<Duration> {
    frozen_since.map(|since| timeout.saturating_sub(now.saturating_sub(since)))
}

/// Only `XDG_RUNTIME_DIR` is private to this user. A lock anywhere shared,
/// like /tmp, could be created first by someone else to stop the tray
/// (pinned by `tests::the_lock_needs_an_absolute_runtime_dir`).
fn runtime_dir(var: Option<OsString>) -> Option<PathBuf> {
    var.map(PathBuf::from).filter(|dir| dir.is_absolute())
}

/// Never through a symlink (pinned by `tests::the_lock_is_not_opened_through_a_symlink`).
fn open_lock(runtime: &Path) -> io::Result<File> {
    File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(runtime.join("waydroid-tray.lock"))
}

fn hold_lock_or_exit() -> File {
    let Some(runtime) = runtime_dir(std::env::var_os("XDG_RUNTIME_DIR")) else {
        eprintln!(
            "XDG_RUNTIME_DIR isn't set to an absolute path, so there's nowhere to keep the lock"
        );
        std::process::exit(1);
    };
    let lock = open_lock(&runtime).expect("create lock file");
    if lock.try_lock().is_err() {
        eprintln!("waydroid-tray is already running");
        std::process::exit(1);
    }
    lock
}

/// Counts how long the session has been frozen, for "Stop session after 30
/// min idle".
struct IdleTimer {
    timeout: Duration,
    frozen_since: Option<Duration>,
    armed: bool,
}

impl IdleTimer {
    fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            frozen_since: None,
            armed: false,
        }
    }

    /// Turning auto-stop on gives a session that's already frozen a full
    /// timeout from now (pinned by `tests::turning_auto_stop_on_restarts_the_countdown`,
    /// `tests::auto_stop_left_on_keeps_counting_down`,
    /// `harness::auto_stop::arming_the_toggle_restarts_the_clock`).
    fn set_armed(&mut self, armed: bool, now: Duration) {
        if armed && !self.armed {
            self.frozen_since = self.frozen_since.map(|_| now);
        }
        self.armed = armed;
    }

    fn observe(&mut self, state: State, now: Duration) {
        self.frozen_since = match state {
            State::Frozen => self.frozen_since.or(Some(now)),
            _ => None,
        };
    }

    /// `None` while auto-stop is off (pinned by
    /// `tests::auto_stop_off_never_counts_down`).
    fn left(&self, now: Duration) -> Option<Duration> {
        if !self.armed {
            return None;
        }
        idle_left(self.frozen_since, now, self.timeout)
    }

    fn fired(&mut self) {
        self.frozen_since = None;
    }
}

async fn dbus_proxy(bus: &Connection) -> DBusProxy<'static> {
    DBusProxy::new(bus)
        .await
        .expect("create org.freedesktop.DBus proxy")
}

async fn watch_owner(dbus: &DBusProxy<'_>, name: &str) -> NameOwnerChangedStream {
    dbus.receive_name_owner_changed_with_args(&[(0, name)])
        .await
        .unwrap_or_else(|err| panic!("watch {name}: {err}"))
}

async fn has_owner(dbus: &DBusProxy<'_>, name: &'static str) -> bool {
    let name = BusName::try_from(name).expect("valid bus name");
    dbus.name_has_owner(name).await.unwrap_or(false)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    print_flags_and_exit().await;
    let home = home();
    let _lock = hold_lock_or_exit();
    let apps_dir = home.join(".local/share/applications");
    let icons_dir = home.join(".local/share/waydroid/data/icons");
    let config_path = Config::path(&home);

    let system = Connection::system()
        .await
        .expect("connect to the system bus");
    let session = Connection::session()
        .await
        .expect("connect to the session bus");
    let system_dbus = dbus_proxy(&system).await;
    let session_dbus = dbus_proxy(&session).await;
    // Subscribe before the first read, so no change slips in between.
    let mut container_changes = watch_owner(&system_dbus, CONTAINER_NAME).await;
    let mut session_changes = watch_owner(&session_dbus, SESSION_NAME).await;
    let poke = Arc::new(Notify::new());
    let config = Config::load(&config_path);
    let settings = Settings::new(config, config_path);
    let auto_stop = settings.auto_stop();

    let mut session_up = has_owner(&session_dbus, SESSION_NAME).await;
    let mut container_up = has_owner(&system_dbus, CONTAINER_NAME).await;
    let mut shown = Snapshot {
        state: read_state(&system).await.with_session(session_up),
        container_up,
        apps: list_apps(&apps_dir, &icons_dir),
    };
    if config.start_at_login && !session_up {
        spawn_waydroid(&["session", "start"], session.clone());
    }

    let tray = WaydroidTray::new(
        shown.clone(),
        home.join(".local/share/icons")
            .to_string_lossy()
            .into_owned(),
        settings,
        system.clone(),
        session.clone(),
        poke.clone(),
    );
    // gamescope Game Mode has no tray host: wait for one instead of panicking into a restart loop.
    let handle = tray
        .assume_sni_available(true)
        .spawn()
        .await
        .expect("register the tray icon");

    let mut interval = tokio::time::interval(POLL);
    let mut idle = IdleTimer::new(idle_timeout());
    loop {
        let state = shown.state;
        let polling = session_up || state == State::Stuck;
        idle.set_armed(auto_stop.load(Ordering::Relaxed), since_boot());
        let idle_left = idle.left(since_boot());
        let new_state = tokio::select! {
            _ = interval.tick() => if polling { read_state(&system).await } else { state },
            _ = tokio::time::sleep(STARTING_POLL), if state == State::Starting => read_state(&system).await,
            _ = tokio::time::sleep(idle_left.unwrap_or_default()), if idle_left.is_some() => {
                spawn_waydroid(&["session", "stop"], session.clone());
                idle.fired();
                state
            }
            _ = poke.notified() => {
                tokio::time::sleep(AFTER_MENU_ACTION).await;
                read_state(&system).await
            }
            Some(change) = container_changes.next() => {
                container_up = gained_owner(&change);
                if container_up { read_state(&system).await } else { State::Stopped }
            }
            Some(change) = session_changes.next() => {
                session_up = gained_owner(&change);
                let state = read_state(&system).await;
                if session_up { state } else { state.without_session_manager() }
            }
        };
        let new_state = new_state.with_session(session_up);
        idle.observe(new_state, since_boot());
        let now = Snapshot {
            state: new_state,
            container_up,
            apps: list_apps(&apps_dir, &icons_dir),
        };
        if now == shown {
            continue;
        }
        shown = now.clone();
        handle.update(move |tray| tray.show(now)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(30 * 60);

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn a_running_session_has_no_deadline() {
        assert_eq!(idle_left(None, secs(1_000), TIMEOUT), None);
    }

    #[test]
    fn the_deadline_counts_down_while_frozen() {
        assert_eq!(
            idle_left(Some(secs(100)), secs(700), TIMEOUT),
            Some(secs(1_200))
        );
    }

    /// Frozen two minutes before a thirteen-hour suspend.
    #[test]
    fn a_session_frozen_across_a_suspend_is_due_on_resume() {
        let froze = secs(120);
        let resumed = froze + secs(13 * 60 * 60);
        assert_eq!(idle_left(Some(froze), resumed, TIMEOUT), Some(secs(0)));
    }

    #[test]
    fn a_clock_that_jumps_backwards_does_not_panic() {
        assert_eq!(
            idle_left(Some(secs(700)), secs(100), TIMEOUT),
            Some(TIMEOUT)
        );
    }

    fn frozen_at_zero() -> IdleTimer {
        let mut idle = IdleTimer::new(TIMEOUT);
        idle.observe(State::Frozen, secs(0));
        idle
    }

    #[test]
    fn turning_auto_stop_on_restarts_the_countdown() {
        let mut idle = frozen_at_zero();
        idle.set_armed(true, secs(3_600));
        assert_eq!(idle.left(secs(3_600)), Some(TIMEOUT));
    }

    #[test]
    fn auto_stop_left_on_keeps_counting_down() {
        let mut idle = frozen_at_zero();
        idle.set_armed(true, secs(0));
        idle.set_armed(true, secs(600));
        assert_eq!(idle.left(secs(600)), Some(TIMEOUT - secs(600)));
    }

    #[test]
    fn auto_stop_off_never_counts_down() {
        let idle = frozen_at_zero();
        assert_eq!(idle.left(secs(3_600)), None);
    }

    #[test]
    fn the_lock_needs_an_absolute_runtime_dir() {
        assert_eq!(runtime_dir(None), None);
        assert_eq!(runtime_dir(Some("".into())), None);
        assert_eq!(runtime_dir(Some("run/user/1000".into())), None);
        assert_eq!(
            runtime_dir(Some("/run/user/1000".into())),
            Some(PathBuf::from("/run/user/1000"))
        );
    }

    #[test]
    fn the_lock_is_not_opened_through_a_symlink() {
        let dir = test_support::Scratch::new("lock");
        std::os::unix::fs::symlink(dir.join("target"), dir.join("waydroid-tray.lock")).unwrap();
        assert!(open_lock(&dir).is_err());
        assert!(!dir.join("target").exists());
    }

    #[test]
    fn since_boot_outlasts_this_process() {
        let boot = since_boot();
        assert!(boot > Duration::ZERO, "{boot:?}");
        assert!(since_boot() >= boot);
    }
}
