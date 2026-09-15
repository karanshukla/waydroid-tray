//! Runs the real tray against private system and session buses, with mock
//! Waydroid, notification and tray host services, and a `waydroid` shim on
//! PATH. Needs `busd` on PATH (`cargo install busd`), nothing else.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, interface};

const CONTAINER_NAME: &str = "id.waydro.Container";
const SESSION_NAME: &str = "id.waydro.Session";
/// Well under the tray's 5s poll, so passing means a signal did it.
const QUICK: Duration = Duration::from_secs(2);

const BUS_CONFIG: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow own="*"/>
    <allow send_destination="*"/>
  </policy>
</busconfig>
"#;

/// Logs its arguments, and fails if a `fail` file exists next to its dir.
const SHIM: &str = r#"#!/bin/sh
root=$(dirname "$0")/..
echo "$*" >> "$root/waydroid.log"
if [ -e "$root/fail" ]; then
    echo "shim: $* failed" >&2
    exit 1
fi
"#;

#[derive(Default)]
struct Mock {
    /// What `GetSession` reports as `state`; `None` means no session.
    session_state: Option<&'static str>,
    get_session_calls: usize,
    container_calls: Vec<&'static str>,
    /// Make `Freeze`/`Unfreeze` fail like an uninitialized Waydroid.
    container_fails: bool,
    notifications: Vec<(String, String)>,
    tray_item: Option<String>,
}

type Shared = Arc<Mutex<Mock>>;

struct ContainerManager(Shared);

impl ContainerManager {
    fn record(&self, method: &'static str) -> zbus::fdo::Result<()> {
        let mut mock = self.0.lock().unwrap();
        mock.container_calls.push(method);
        match mock.container_fails {
            true => Err(zbus::fdo::Error::Failed("WayDroid is not initialized".into())),
            false => Ok(()),
        }
    }
}

#[interface(name = "id.waydro.ContainerManager")]
impl ContainerManager {
    fn get_session(&self) -> HashMap<String, String> {
        let mut mock = self.0.lock().unwrap();
        mock.get_session_calls += 1;
        mock.session_state.map(|state| HashMap::from([("state".into(), state.into())])).unwrap_or_default()
    }

    fn freeze(&self) -> zbus::fdo::Result<()> {
        self.record("Freeze")
    }

    fn unfreeze(&self) -> zbus::fdo::Result<()> {
        self.record("Unfreeze")
    }
}

struct Notifications(Shared);

#[interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        _app_name: String,
        _replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        _hints: HashMap<String, OwnedValue>,
        _expire_timeout: i32,
    ) -> u32 {
        let mut mock = self.0.lock().unwrap();
        mock.notifications.push((summary, body));
        mock.notifications.len() as u32
    }
}

/// Stands in for Plasma, which the tray has to register with to start.
struct Watcher(Shared);

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: String) {
        self.0.lock().unwrap().tray_item = Some(service);
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

struct Harness {
    dir: PathBuf,
    processes: Vec<Child>,
    system_address: String,
    session_address: String,
    system: Connection,
    session: Connection,
    mock: Shared,
}

impl Harness {
    async fn new() -> Self {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let name = format!("waydroid-tray-harness-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
        let dir = std::env::temp_dir().join(name);
        for sub in ["bin", "home", "config", "run"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        fs::write(dir.join("bus.conf"), BUS_CONFIG).unwrap();
        let shim = dir.join("bin/waydroid");
        fs::write(&shim, SHIM).unwrap();
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();

        let mut processes = Vec::new();
        let system_address = start_bus(&dir, "system", &mut processes);
        let session_address = start_bus(&dir, "session", &mut processes);
        let mock = Shared::default();
        let system = zbus::connection::Builder::address(&*system_address)
            .unwrap()
            .serve_at("/ContainerManager", ContainerManager(mock.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        let session = zbus::connection::Builder::address(&*session_address)
            .unwrap()
            .serve_at("/org/freedesktop/Notifications", Notifications(mock.clone()))
            .unwrap()
            .serve_at("/StatusNotifierWatcher", Watcher(mock.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        session.request_name("org.freedesktop.Notifications").await.unwrap();
        session.request_name("org.kde.StatusNotifierWatcher").await.unwrap();
        Harness { dir, processes, system_address, session_address, system, session, mock }
    }

    fn mock(&self) -> std::sync::MutexGuard<'_, Mock> {
        self.mock.lock().unwrap()
    }

    /// Container service up, no session yet.
    async fn start_container(&self) {
        self.system.request_name(CONTAINER_NAME).await.unwrap();
    }

    /// Session up in `state`, in the order Waydroid does it.
    async fn start_session(&self, state: &'static str) {
        self.start_container().await;
        self.mock().session_state = Some(state);
        self.session.request_name(SESSION_NAME).await.unwrap();
    }

    fn write_config(&self, text: &str) {
        let path = self.dir.join("config/waydroid-tray/config");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn fail_waydroid(&self) {
        fs::write(self.dir.join("fail"), "").unwrap();
    }

    fn waydroid_log(&self) -> Vec<String> {
        let text = fs::read_to_string(self.dir.join("waydroid.log")).unwrap_or_default();
        text.lines().map(str::to_owned).collect()
    }

    /// Starts the tray and waits for it to register with the watcher.
    async fn start_tray(&mut self) {
        let path = format!("{}:{}", self.dir.join("bin").display(), std::env::var("PATH").unwrap());
        let tray = Command::new(env!("CARGO_BIN_EXE_waydroid-tray"))
            .env("HOME", self.dir.join("home"))
            .env("XDG_CONFIG_HOME", self.dir.join("config"))
            .env("XDG_RUNTIME_DIR", self.dir.join("run"))
            .env("TMPDIR", self.dir.join("run"))
            .env("PATH", path)
            .env("DBUS_SYSTEM_BUS_ADDRESS", &self.system_address)
            .env("DBUS_SESSION_BUS_ADDRESS", &self.session_address)
            .spawn()
            .unwrap();
        self.processes.push(tray);
        wait_for("the tray to register", QUICK, async || self.mock().tray_item.is_some()).await;
    }

    fn tray_running(&mut self) -> bool {
        self.processes.last_mut().unwrap().try_wait().unwrap().is_none()
    }

    fn tray_item(&self) -> String {
        self.mock().tray_item.clone().unwrap()
    }

    async fn tray_call<B>(&self, path: &str, interface: &str, method: &str, body: &B) -> zbus::Message
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.session
            .call_method(Some(self.tray_item()), path, Some(interface), method, body)
            .await
            .unwrap()
    }

    async fn tray_property(&self, name: &str) -> String {
        let reply = self
            .tray_call(
                "/StatusNotifierItem",
                "org.freedesktop.DBus.Properties",
                "Get",
                &("org.kde.StatusNotifierItem", name),
            )
            .await;
        String::try_from(reply.body().deserialize::<OwnedValue>().unwrap()).unwrap()
    }

    async fn icon(&self) -> String {
        self.tray_property("IconName").await
    }

    async fn click(&self, label: &str) {
        let reply = self
            .tray_call("/MenuBar", "com.canonical.dbusmenu", "GetGroupProperties", &(Vec::<i32>::new(), vec!["label"]))
            .await;
        let items: Vec<(i32, HashMap<String, OwnedValue>)> = reply.body().deserialize().unwrap();
        let (id, _) = items
            .iter()
            .find(|(_, props)| props.get("label").and_then(|v| v.downcast_ref::<&str>().ok()) == Some(label))
            .unwrap_or_else(|| panic!("no menu item {label:?}"));
        self.tray_call("/MenuBar", "com.canonical.dbusmenu", "Event", &(*id, "clicked", Value::from(0), 0u32))
            .await;
    }

    async fn middle_click(&self) {
        self.tray_call("/StatusNotifierItem", "org.kde.StatusNotifierItem", "SecondaryActivate", &(0i32, 0i32))
            .await;
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Tray first, then the buses it's connected to.
        for process in self.processes.iter_mut().rev() {
            let _ = process.kill();
            let _ = process.wait();
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Starts a bus and waits until it accepts connections. Returns its address.
fn start_bus(dir: &std::path::Path, name: &str, processes: &mut Vec<Child>) -> String {
    let address = format!("unix:path={}", dir.join(format!("{name}.sock")).display());
    let (mut ready, ready_writer) = std::io::pipe().unwrap();
    let bus = Command::new("busd")
        .arg("--config")
        .arg(dir.join("bus.conf"))
        .args(["--address", &address, "--ready-fd", "1"])
        .stdout(ready_writer)
        .spawn()
        .expect("run busd (install it with `cargo install busd`)");
    processes.push(bus);
    // busd writes READY=1 to its stdout and closes it once it's listening.
    let mut status = String::new();
    ready.read_to_string(&mut status).unwrap();
    assert_eq!(status.trim(), "READY=1", "busd didn't start");
    address
}

async fn wait_for(what: &str, within: Duration, mut check: impl AsyncFnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// #2: session state follows NameOwnerChanged.

#[tokio::test]
async fn session_start_is_picked_up_without_polling() {
    let mut h = Harness::new().await;
    h.start_tray().await;
    assert_eq!(h.icon().await, "waydroid-tray-stopped");
    h.start_session("RUNNING").await;
    wait_for("Running", QUICK, async || h.icon().await == "waydroid-tray-running").await;
}

#[tokio::test]
async fn session_stop_is_immediate() {
    let mut h = Harness::new().await;
    h.start_session("RUNNING").await;
    h.start_tray().await;
    assert_eq!(h.icon().await, "waydroid-tray-running");
    // GetSession would still say RUNNING, so only the signal can do this.
    h.session.release_name(SESSION_NAME).await.unwrap();
    wait_for("Stopped", QUICK, async || h.icon().await == "waydroid-tray-stopped").await;
}

#[tokio::test]
async fn frozen_session_shows_frozen() {
    let mut h = Harness::new().await;
    h.start_session("FROZEN").await;
    h.start_tray().await;
    assert_eq!(h.icon().await, "waydroid-tray-frozen");
}

#[tokio::test]
async fn no_get_session_calls_while_stopped() {
    let mut h = Harness::new().await;
    // The container service can be up with no session, e.g. after a stop.
    h.start_container().await;
    h.start_tray().await;
    let calls = h.mock().get_session_calls;
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(h.mock().get_session_calls, calls);
}

// #3: hide the icon while stopped.

#[tokio::test]
async fn hidden_while_stopped_when_enabled() {
    let mut h = Harness::new().await;
    h.write_config("hide_when_stopped=true\n");
    h.start_tray().await;
    assert_eq!(h.tray_property("Status").await, "Passive");
    h.start_session("RUNNING").await;
    wait_for("Active", QUICK, async || h.tray_property("Status").await == "Active").await;
}

#[tokio::test]
async fn hide_toggle_applies_and_persists() {
    let mut h = Harness::new().await;
    h.start_tray().await;
    assert_eq!(h.tray_property("Status").await, "Active");
    h.click("Hide icon while stopped").await;
    assert_eq!(h.tray_property("Status").await, "Passive");
    let config = fs::read_to_string(h.dir.join("config/waydroid-tray/config")).unwrap();
    assert!(config.contains("hide_when_stopped=true"), "{config}");
}

// #4: start session at login.

#[tokio::test]
async fn start_at_login_starts_a_stopped_session() {
    let mut h = Harness::new().await;
    h.write_config("start_at_login=true\n");
    h.start_tray().await;
    wait_for("session start", QUICK, async || h.waydroid_log() == ["session start"]).await;
}

#[tokio::test]
async fn start_at_login_leaves_a_running_session_alone() {
    let mut h = Harness::new().await;
    h.write_config("start_at_login=true\n");
    h.start_session("RUNNING").await;
    h.start_tray().await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(h.waydroid_log().is_empty(), "{:?}", h.waydroid_log());
}

// #5: freeze and unfreeze.

#[tokio::test]
async fn freeze_when_running() {
    let mut h = Harness::new().await;
    h.start_session("RUNNING").await;
    h.start_tray().await;
    h.click("Freeze").await;
    wait_for("Freeze", QUICK, async || h.mock().container_calls == ["Freeze"]).await;
}

#[tokio::test]
async fn unfreeze_when_frozen() {
    let mut h = Harness::new().await;
    h.start_session("FROZEN").await;
    h.start_tray().await;
    h.click("Unfreeze").await;
    wait_for("Unfreeze", QUICK, async || h.mock().container_calls == ["Unfreeze"]).await;
}

#[tokio::test]
async fn freeze_error_is_notified() {
    let mut h = Harness::new().await;
    h.start_session("RUNNING").await;
    h.mock().container_fails = true;
    h.start_tray().await;
    h.click("Freeze").await;
    wait_for("a notification", QUICK, async || !h.mock().notifications.is_empty()).await;
    let (summary, body) = h.mock().notifications[0].clone();
    assert_eq!(summary, "Freeze failed");
    assert!(body.contains("not initialized"), "{body}");
    assert!(h.tray_running());
}

// #6: notify when a waydroid command fails.

#[tokio::test]
async fn failed_command_is_notified_with_stderr() {
    let mut h = Harness::new().await;
    h.fail_waydroid();
    h.start_tray().await;
    h.click("Start session").await;
    wait_for("a notification", QUICK, async || !h.mock().notifications.is_empty()).await;
    let notifications = h.mock().notifications.clone();
    assert_eq!(
        notifications,
        [("waydroid session start failed".into(), "shim: session start failed".into())]
    );
}

#[tokio::test]
async fn successful_command_is_not_notified() {
    let mut h = Harness::new().await;
    h.start_tray().await;
    h.click("Show full UI").await;
    wait_for("the command", QUICK, async || h.waydroid_log() == ["show-full-ui"]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(h.mock().notifications.is_empty());
}

// #8: middle click toggles the session.

#[tokio::test]
async fn middle_click_starts_a_stopped_session() {
    let mut h = Harness::new().await;
    h.start_tray().await;
    h.middle_click().await;
    wait_for("session start", QUICK, async || h.waydroid_log() == ["session start"]).await;
}

#[tokio::test]
async fn middle_click_stops_a_running_session() {
    let mut h = Harness::new().await;
    h.start_session("RUNNING").await;
    h.start_tray().await;
    h.middle_click().await;
    wait_for("session stop", QUICK, async || h.waydroid_log() == ["session stop"]).await;
}
