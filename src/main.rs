//! System tray icon for Waydroid: session status, start/stop, and app launcher.

mod apps;
mod bus;
mod cli;
mod config;
mod notify;
mod state;
mod tray;

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use ksni::TrayMethods;
use tokio::sync::Notify;
use zbus::Connection;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;

use apps::list_apps;
use bus::{CONTAINER_NAME, SESSION_NAME, gained_owner, read_state};
use cli::spawn_waydroid;
use config::{Config, Settings};
use state::State;
use tray::WaydroidTray;

/// Refreshes the app list, and Running vs Frozen while a session exists
/// (freezing doesn't show up on the bus).
const POLL: Duration = Duration::from_secs(5);
/// Waydroid claims its session name before it asks the container to start,
/// so right after the name appears the container still reports no session.
/// Re-check this often until it does.
const STARTING_POLL: Duration = Duration::from_millis(500);
/// How long to wait after a menu action before re-reading the state.
const SETTLE: Duration = Duration::from_millis(1500);
/// How long a session has to stay frozen before "Stop session after 30 min
/// idle" stops it. Waydroid freezes the container when no Android windows are
/// open, so this only ever fires on a session nothing is using.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Overrides `IDLE_TIMEOUT`, in seconds. The tests can't wait half an hour.
const IDLE_TIMEOUT_VAR: &str = "WAYDROID_TRAY_IDLE_SECS";

fn idle_timeout() -> Duration {
    std::env::var(IDLE_TIMEOUT_VAR)
        .ok()
        .and_then(|secs| secs.parse().ok())
        .map_or(IDLE_TIMEOUT, Duration::from_secs)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
    let runtime =
        std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let apps_dir = home.join(".local/share/applications");
    let config_path = Config::path(&home);

    let lock = File::create(runtime.join("waydroid-tray.lock")).expect("create lock file");
    if lock.try_lock().is_err() {
        eprintln!("waydroid-tray is already running");
        std::process::exit(1);
    }

    let system = Connection::system()
        .await
        .expect("connect to the system bus");
    let session = Connection::session()
        .await
        .expect("connect to the session bus");
    let system_dbus = DBusProxy::new(&system)
        .await
        .expect("create org.freedesktop.DBus proxy");
    let session_dbus = DBusProxy::new(&session)
        .await
        .expect("create org.freedesktop.DBus proxy");
    // Subscribe before the first read, so no change slips in between.
    let mut container_changes = system_dbus
        .receive_name_owner_changed_with_args(&[(0, CONTAINER_NAME)])
        .await
        .expect("watch the container manager");
    let mut session_changes = session_dbus
        .receive_name_owner_changed_with_args(&[(0, SESSION_NAME)])
        .await
        .expect("watch the session manager");
    let poke = Arc::new(Notify::new());
    let config = Config::load(&config_path);
    let settings = Settings::new(config, config_path);
    let auto_stop = settings.auto_stop();

    let session_name = BusName::try_from(SESSION_NAME).expect("valid bus name");
    let mut session_up = session_dbus
        .name_has_owner(session_name)
        .await
        .unwrap_or(false);
    let container_name = BusName::try_from(CONTAINER_NAME).expect("valid bus name");
    let mut container_up = system_dbus
        .name_has_owner(container_name)
        .await
        .unwrap_or(false);
    let mut state = read_state(&system).await.with_session(session_up);
    if config.start_at_login && !session_up {
        spawn_waydroid(&["session", "start"], session.clone());
    }

    let mut apps = list_apps(&apps_dir);
    let mut tray = WaydroidTray::new(
        state,
        home.join(".local/share/icons")
            .to_string_lossy()
            .into_owned(),
        settings,
        system.clone(),
        session.clone(),
        poke.clone(),
    );
    tray.container_up = container_up;
    tray.set_apps(apps.clone());
    let handle = tray.spawn().await.expect("register the tray icon");

    let mut interval = tokio::time::interval(POLL);
    let idle_after = idle_timeout();
    let mut frozen_since: Option<Instant> = None;
    let mut was_armed = false;
    loop {
        let polling = session_up || state == State::Stuck;
        let was_container_up = container_up;
        let armed = auto_stop.load(Ordering::Relaxed);
        if armed && !was_armed {
            frozen_since = frozen_since.map(|_| Instant::now());
        }
        was_armed = armed;
        let idle_left = frozen_since.map(|since| idle_after.saturating_sub(since.elapsed()));
        let new_state = tokio::select! {
            _ = interval.tick() => if polling { read_state(&system).await } else { state },
            _ = tokio::time::sleep(STARTING_POLL), if state == State::Starting => read_state(&system).await,
            _ = tokio::time::sleep(idle_left.unwrap_or_default()), if armed && idle_left.is_some() => {
                spawn_waydroid(&["session", "stop"], session.clone());
                frozen_since = None;
                state
            }
            _ = poke.notified() => {
                tokio::time::sleep(SETTLE).await;
                read_state(&system).await
            }
            Some(change) = container_changes.next() => {
                container_up = gained_owner(&change);
                if container_up { read_state(&system).await } else { State::Stopped }
            }
            Some(change) = session_changes.next() => {
                session_up = gained_owner(&change);
                let state = read_state(&system).await;
                if !session_up && state != State::Stopped { State::Stuck } else { state }
            }
        };
        let new_state = new_state.with_session(session_up);
        frozen_since = match new_state {
            State::Frozen => frozen_since.or_else(|| Some(Instant::now())),
            _ => None,
        };
        let new_apps = list_apps(&apps_dir);
        if new_state == state && new_apps == apps && container_up == was_container_up {
            continue;
        }
        state = new_state;
        apps = new_apps;
        let apps_for_tray = apps.clone();
        let container_up_now = container_up;
        handle
            .update(move |tray| {
                tray.state = state;
                tray.container_up = container_up_now;
                tray.set_apps(apps_for_tray);
            })
            .await;
    }
}
