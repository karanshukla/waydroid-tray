//! System tray icon for Waydroid: session status, start/stop, and app launcher.

use std::collections::HashMap;
use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use ksni::menu::{MenuItem, StandardItem, SubMenu};
use ksni::{ToolTip, TrayMethods};
use tokio::sync::Notify;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::Connection;

const BUS_NAME: &str = "id.waydro.Container";
const OBJECT_PATH: &str = "/ContainerManager";
const INTERFACE: &str = "id.waydro.ContainerManager";
const POLL: Duration = Duration::from_secs(5);
/// How long to wait after a menu action before re-reading the state.
const SETTLE: Duration = Duration::from_millis(1500);

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
    poke: Arc<Notify>,
}

impl WaydroidTray {
    fn run(&self, args: &[&str]) {
        spawn_waydroid(args);
        self.poke.notify_one();
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

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let stopped = self.state == State::Stopped;
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
            StandardItem {
                label: "Quit tray".into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Reads the session state without ever starting the container service.
async fn read_state(conn: &Connection, dbus: &DBusProxy<'_>) -> State {
    // Only talk to the container manager if it already owns its name.
    // Addressing the well-known name directly (like `waydroid status` does)
    // triggers D-Bus activation, which would start waydroid-container.service
    // on every poll.
    let name = BusName::try_from(BUS_NAME).expect("valid bus name");
    if !dbus.name_has_owner(name.clone()).await.unwrap_or(false) {
        return State::Stopped;
    }
    let Ok(owner) = dbus.get_name_owner(name).await else {
        return State::Stopped;
    };
    let Ok(reply) = conn
        .call_method(Some(owner.into_inner()), OBJECT_PATH, Some(INTERFACE), "GetSession", &())
        .await
    else {
        return State::Stopped;
    };
    let session: HashMap<String, String> = reply.body().deserialize().unwrap_or_default();
    match session.get("state").map(String::as_str) {
        None => State::Stopped,
        Some("FROZEN") => State::Frozen,
        Some(_) => State::Running,
    }
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

fn spawn_waydroid(args: &[&str]) {
    let child = Command::new("waydroid")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Own process group, so quitting the tray doesn't take a session with it.
        .process_group(0)
        .spawn();
    match child {
        // Reap it in the background so finished commands don't linger as zombies.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(err) => eprintln!("failed to run waydroid {}: {err}", args.join(" ")),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let apps_dir = home.join(".local/share/applications");

    let lock = File::create(runtime.join("waydroid-tray.lock")).expect("create lock file");
    if lock.try_lock().is_err() {
        eprintln!("waydroid-tray is already running");
        std::process::exit(1);
    }

    let conn = Connection::system().await.expect("connect to the system bus");
    let dbus = DBusProxy::new(&conn).await.expect("create org.freedesktop.DBus proxy");
    let poke = Arc::new(Notify::new());

    let mut state = read_state(&conn, &dbus).await;
    let mut apps = list_apps(&apps_dir);
    let mut tray = WaydroidTray {
        state,
        apps: Vec::new(),
        icon_theme_path: home.join(".local/share/icons").to_string_lossy().into_owned(),
        poke: poke.clone(),
    };
    tray.set_apps(apps.clone());
    let handle = tray.spawn().await.expect("register the tray icon");

    // Waydroid doesn't signal session changes, so poll. Menu actions poke the
    // loop to refresh sooner.
    let mut interval = tokio::time::interval(POLL);
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = poke.notified() => tokio::time::sleep(SETTLE).await,
        }
        let new_state = read_state(&conn, &dbus).await;
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
