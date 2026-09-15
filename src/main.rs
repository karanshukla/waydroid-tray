//! System tray icon for Waydroid: session status, start/stop, and app launcher.

mod config;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
use ksni::{ToolTip, TrayMethods};
use tokio::process::Command;
use tokio::sync::Notify;
use zbus::fdo::{DBusProxy, NameOwnerChanged};
use zbus::names::{BusName, OwnedUniqueName};
use zbus::zvariant::Value;
use zbus::Connection;

use config::Config;

const CONTAINER_NAME: &str = "id.waydro.Container";
const SESSION_NAME: &str = "id.waydro.Session";
const OBJECT_PATH: &str = "/ContainerManager";
const INTERFACE: &str = "id.waydro.ContainerManager";
/// Refreshes the app list, and Running vs Frozen while a session exists
/// (freezing doesn't show up on the bus).
const POLL: Duration = Duration::from_secs(5);
/// Waydroid claims its session name before it asks the container to start,
/// so right after the name appears the container still reports no session.
/// Re-check this often until it does.
const STARTING_POLL: Duration = Duration::from_millis(500);
/// How long to wait after a menu action before re-reading the state.
const SETTLE: Duration = Duration::from_millis(1500);
/// `session start` only returns when the session ends, so it only counts as
/// failed if it exits within this long.
const START_GRACE: Duration = Duration::from_secs(5);
/// How much of a failed command's stderr to show.
const ERROR_LINES: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Stopped,
    Running,
    Frozen,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            State::Stopped => "Stopped",
            State::Running => "Running",
            State::Frozen => "Frozen (idle)",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            State::Stopped => "waydroid-tray-stopped",
            State::Running => "waydroid-tray-running",
            State::Frozen => "waydroid-tray-frozen",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct AppEntry {
    name: String,
    package: String,
    icon: PathBuf,
}

struct WaydroidTray {
    state: State,
    apps: Vec<(AppEntry, Vec<u8>)>,
    icon_theme_path: String,
    config: Config,
    config_path: PathBuf,
    system: Connection,
    session: Connection,
    poke: Arc<Notify>,
}

impl WaydroidTray {
    fn run(&self, args: &[&str]) {
        spawn_waydroid(args, self.session.clone());
        self.poke.notify_one();
    }

    /// Calls a container manager method that takes no arguments, e.g. `Freeze`.
    fn call(&self, method: &'static str) {
        let (system, session, poke) = (self.system.clone(), self.session.clone(), self.poke.clone());
        tokio::spawn(async move {
            if let Some(owner) = container_owner(&system).await {
                let reply = system
                    .call_method(Some(owner), OBJECT_PATH, Some(INTERFACE), method, &())
                    .await;
                if let Err(err) = reply {
                    notify_failure(&session, &format!("{method} failed"), &err.to_string()).await;
                }
            }
            poke.notify_one();
        });
    }

    fn save_config(&self) {
        if let Err(err) = self.config.save(&self.config_path) {
            eprintln!("failed to save {}: {err}", self.config_path.display());
        }
    }

    fn set_apps(&mut self, apps: Vec<AppEntry>) {
        self.apps = apps
            .into_iter()
            .map(|app| {
                let png = fs::read(&app.icon).unwrap_or_default();
                (app, png)
            })
            .collect();
    }
}

impl ksni::Tray for WaydroidTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        "Waydroid".into()
    }

    fn status(&self) -> ksni::Status {
        // Plasma tucks passive items away in the hidden icons.
        if self.config.hide_when_stopped && self.state == State::Stopped {
            ksni::Status::Passive
        } else {
            ksni::Status::Active
        }
    }

    fn icon_name(&self) -> String {
        self.state.icon().into()
    }

    fn icon_theme_path(&self) -> String {
        self.icon_theme_path.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: format!("Waydroid: {}", self.state.label()),
            ..Default::default()
        }
    }

    /// Middle click toggles the session.
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        if self.state == State::Stopped {
            self.run(&["session", "start"]);
        } else {
            self.run(&["session", "stop"]);
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let stopped = self.state == State::Stopped;
        // Also the container manager method to call.
        let freeze = if self.state == State::Frozen { "Unfreeze" } else { "Freeze" };
        let apps: Vec<MenuItem<Self>> = self
            .apps
            .iter()
            .map(|(app, png)| {
                let package = app.package.clone();
                StandardItem {
                    label: app.name.clone(),
                    icon_data: png.clone(),
                    activate: Box::new(move |tray: &mut Self| {
                        tray.run(&["app", "launch", &package])
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();

        vec![
            StandardItem {
                label: format!("Waydroid: {}", self.state.label()),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Start session".into(),
                enabled: stopped,
                activate: Box::new(|tray: &mut Self| tray.run(&["session", "start"])),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Stop session".into(),
                enabled: !stopped,
                activate: Box::new(|tray: &mut Self| tray.run(&["session", "stop"])),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: freeze.into(),
                enabled: !stopped,
                activate: Box::new(move |tray: &mut Self| tray.call(freeze)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Show full UI".into(),
                activate: Box::new(|tray: &mut Self| tray.run(&["show-full-ui"])),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Apps".into(),
                enabled: !apps.is_empty(),
                submenu: apps,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Start session at login".into(),
                checked: self.config.start_at_login,
                activate: Box::new(|tray: &mut Self| {
                    tray.config.start_at_login ^= true;
                    tray.save_config();
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Hide icon while stopped".into(),
                checked: self.config.hide_when_stopped,
                activate: Box::new(|tray: &mut Self| {
                    tray.config.hide_when_stopped ^= true;
                    tray.save_config();
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit tray".into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// The container manager's unique bus name, if it's running. Asking the bus
/// who owns a name never activates anything.
async fn container_owner(system: &Connection) -> Option<OwnedUniqueName> {
    let dbus = DBusProxy::new(system).await.ok()?;
    let name = BusName::try_from(CONTAINER_NAME).expect("valid bus name");
    dbus.get_name_owner(name).await.ok()
}

/// Reads the session state without ever starting the container service.
async fn read_state(system: &Connection) -> State {
    // Only talk to the container manager if it already owns its name, and
    // then via its unique name. Addressing the well-known name directly (like
    // `waydroid status` does) triggers D-Bus activation, which would start
    // waydroid-container.service on every poll.
    let Some(owner) = container_owner(system).await else {
        return State::Stopped;
    };
    let Ok(reply) = system
        .call_method(Some(owner), OBJECT_PATH, Some(INTERFACE), "GetSession", &())
        .await
    else {
        return State::Stopped;
    };
    let session: HashMap<String, String> = reply.body().deserialize().unwrap_or_default();
    match session.get("state").map(String::as_str) {
        None | Some("STOPPED") => State::Stopped,
        Some("FROZEN") => State::Frozen,
        Some(_) => State::Running,
    }
}

fn gained_owner(change: &NameOwnerChanged) -> bool {
    change.args().is_ok_and(|args| args.new_owner.is_some())
}

/// Visible Waydroid apps, read from the launchers Waydroid generates.
fn list_apps(dir: &PathBuf) -> Vec<AppEntry> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut apps: Vec<AppEntry> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().into_string().ok()?;
            let package = file.strip_prefix("waydroid.")?.strip_suffix(".desktop")?.to_owned();
            let text = fs::read_to_string(entry.path()).ok()?;
            let fields = desktop_entry(&text);
            if fields.get("NoDisplay").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
                return None;
            }
            Some(AppEntry {
                name: fields.get("Name").cloned().unwrap_or_else(|| package.clone()),
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

/// Runs `waydroid` in the background and shows a notification if it fails.
fn spawn_waydroid(args: &[&str], session: Connection) {
    let command = format!("waydroid {}", args.join(" "));
    let starts_session = args == ["session", "start"];
    // A file rather than a pipe: the session outlives `session start`'s grace
    // period, and would get EPIPE writing to a pipe after the tray quits.
    let stderr = scratch_file();
    let child = Command::new("waydroid")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr.as_ref().and_then(|file| file.try_clone().ok()).map_or_else(Stdio::null, Stdio::from))
        // Own process group, so quitting the tray doesn't take a session with it.
        .process_group(0)
        .spawn();
    tokio::spawn(async move {
        let summary = format!("{command} failed");
        let status = match child {
            Err(err) => return notify_failure(&session, &summary, &err.to_string()).await,
            Ok(mut child) if starts_session => {
                match tokio::time::timeout(START_GRACE, child.wait()).await {
                    // Still going, so the session started. Reap it when it ends.
                    Err(_) => return drop(child.wait().await),
                    // A started session doesn't return, so exiting this soon is
                    // a failure even with status 0, which is what Waydroid exits
                    // with when the container isn't listening.
                    Ok(status) => status,
                }
            }
            Ok(mut child) => match child.wait().await {
                Ok(status) if status.success() => return,
                status => status,
            },
        };
        let body = stderr.and_then(last_lines).unwrap_or_else(|| match status {
            Ok(status) => status.to_string(),
            Err(err) => err.to_string(),
        });
        notify_failure(&session, &summary, &body).await;
    });
}

/// An already-deleted temp file, to capture a child's stderr in.
fn scratch_file() -> Option<File> {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let name = format!("waydroid-tray-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
    let path = std::env::temp_dir().join(name);
    let file = File::options().read(true).write(true).create_new(true).open(&path).ok()?;
    fs::remove_file(&path).ok()?;
    Some(file)
}

fn last_lines(mut file: File) -> Option<String> {
    let mut text = String::new();
    file.rewind().ok()?;
    file.read_to_string(&mut text).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(ERROR_LINES)..].join("\n");
    (!tail.is_empty()).then_some(tail)
}

/// Autostarted, the tray has no terminal, so failures go to a notification.
async fn notify_failure(session: &Connection, summary: &str, body: &str) {
    let hints: HashMap<&str, Value> = HashMap::new();
    let reply = session
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &("Waydroid", 0u32, "waydroid", summary, body, Vec::<&str>::new(), hints, -1i32),
        )
        .await;
    if let Err(err) = reply {
        eprintln!("{summary}: {body} (notification failed: {err})");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let apps_dir = home.join(".local/share/applications");
    let config_path = Config::path(&home);

    let lock = File::create(runtime.join("waydroid-tray.lock")).expect("create lock file");
    if lock.try_lock().is_err() {
        eprintln!("waydroid-tray is already running");
        std::process::exit(1);
    }

    let system = Connection::system().await.expect("connect to the system bus");
    let session = Connection::session().await.expect("connect to the session bus");
    let system_dbus = DBusProxy::new(&system).await.expect("create org.freedesktop.DBus proxy");
    let session_dbus = DBusProxy::new(&session).await.expect("create org.freedesktop.DBus proxy");
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

    let session_name = BusName::try_from(SESSION_NAME).expect("valid bus name");
    let mut session_up = session_dbus.name_has_owner(session_name).await.unwrap_or(false);
    let mut state = read_state(&system).await;
    if config.start_at_login && !session_up {
        spawn_waydroid(&["session", "start"], session.clone());
    }

    let mut apps = list_apps(&apps_dir);
    let mut tray = WaydroidTray {
        state,
        apps: Vec::new(),
        icon_theme_path: home.join(".local/share/icons").to_string_lossy().into_owned(),
        config,
        config_path,
        system: system.clone(),
        session,
        poke: poke.clone(),
    };
    tray.set_apps(apps.clone());
    let handle = tray.spawn().await.expect("register the tray icon");

    // Session start and stop show up as the session manager's bus name coming
    // and going. Freezing doesn't, so poll for that while a session exists.
    // Menu actions poke the loop to refresh sooner.
    let mut interval = tokio::time::interval(POLL);
    loop {
        let starting = session_up && state == State::Stopped;
        let new_state = tokio::select! {
            _ = interval.tick() => if session_up { read_state(&system).await } else { state },
            _ = tokio::time::sleep(STARTING_POLL), if starting => read_state(&system).await,
            _ = poke.notified() => {
                tokio::time::sleep(SETTLE).await;
                read_state(&system).await
            }
            Some(change) = container_changes.next() => {
                if gained_owner(&change) { read_state(&system).await } else { State::Stopped }
            }
            Some(change) = session_changes.next() => {
                session_up = gained_owner(&change);
                if session_up { read_state(&system).await } else { State::Stopped }
            }
        };
        let new_apps = list_apps(&apps_dir);
        if new_state == state && new_apps == apps {
            continue;
        }
        state = new_state;
        apps = new_apps;
        let apps_for_tray = apps.clone();
        handle
            .update(move |tray| {
                tray.state = state;
                tray.set_apps(apps_for_tray);
            })
            .await;
    }
}
