use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use zbus::Connection;
use zbus::zvariant::{OwnedValue, Value};

use crate::mock::{ContainerManager, Mock, Notifications, Shared, Systemd, Watcher};

const CONTAINER_NAME: &str = "id.waydro.Container";
pub const SESSION_NAME: &str = "id.waydro.Session";
const SYSTEMD_NAME: &str = "org.freedesktop.systemd1";
/// Well under the tray's 5s poll, so passing means a signal did it.
pub const QUICK: Duration = Duration::from_secs(2);
/// Stands in for the tray's 30 minute idle timeout.
pub const IDLE_SECS: u64 = 1;

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

/// Logs its arguments. If a `fail` file exists next to its dir, it fails with
/// the exit code in it. Otherwise `session start` keeps running like a real
/// session, and records its pid so the harness can clean it up.
const SHIM: &str = r#"#!/bin/sh
root=$(dirname "$0")/..
echo "$*" >> "$root/waydroid.log"
if [ -e "$root/fail" ]; then
    echo "shim: $* failed" >&2
    exit "$(cat "$root/fail")"
fi
if [ "$*" = "session start" ]; then
    echo $$ >> "$root/pids"
    exec sleep 60
fi
"#;

pub struct Harness {
    pub dir: PathBuf,
    processes: Vec<Child>,
    system_address: String,
    session_address: String,
    system: Connection,
    pub session: Connection,
    mock: Shared,
}

impl Harness {
    pub async fn new() -> Self {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "waydroid-tray-harness-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        );
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
            .serve_at("/org/freedesktop/systemd1", Systemd(mock.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        let session = zbus::connection::Builder::address(&*session_address)
            .unwrap()
            .serve_at(
                "/org/freedesktop/Notifications",
                Notifications(mock.clone()),
            )
            .unwrap()
            .serve_at("/StatusNotifierWatcher", Watcher(mock.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        // systemd is always up on a real box, unlike the container service.
        system.request_name(SYSTEMD_NAME).await.unwrap();
        session
            .request_name("org.freedesktop.Notifications")
            .await
            .unwrap();
        session
            .request_name("org.kde.StatusNotifierWatcher")
            .await
            .unwrap();
        Harness {
            dir,
            processes,
            system_address,
            session_address,
            system,
            session,
            mock,
        }
    }

    pub fn mock(&self) -> std::sync::MutexGuard<'_, Mock> {
        self.mock.lock().unwrap()
    }

    /// Container service up, no session yet.
    pub async fn start_container(&self) {
        self.system.request_name(CONTAINER_NAME).await.unwrap();
    }

    /// Session up in `state`, in the order Waydroid does it: the session
    /// manager claims its name first, and only then asks the (already running)
    /// container to start, so for a moment `GetSession` still returns `{}`.
    pub async fn start_session(&self, state: &'static str) {
        self.start_container().await;
        self.session.request_name(SESSION_NAME).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.mock().session_state = Some(state);
    }

    /// Session stopped cleanly: Waydroid clears the container's session
    /// before the session manager drops its name.
    pub async fn stop_session(&self) {
        self.mock().session_state = None;
        self.session.release_name(SESSION_NAME).await.unwrap();
    }

    pub fn write_config(&self, text: &str) {
        let path = self.dir.join("config/waydroid-tray/config");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// Makes every `waydroid` command print an error and exit with `code`.
    pub fn fail_waydroid(&self, code: i32) {
        fs::write(self.dir.join("fail"), code.to_string()).unwrap();
    }

    pub fn waydroid_log(&self) -> Vec<String> {
        let text = fs::read_to_string(self.dir.join("waydroid.log")).unwrap_or_default();
        text.lines().map(str::to_owned).collect()
    }

    /// Starts the tray and waits for it to register with the watcher.
    pub async fn start_tray(&mut self) {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let tray = Command::new(env!("CARGO_BIN_EXE_waydroid-tray"))
            .env("HOME", self.dir.join("home"))
            .env("XDG_CONFIG_HOME", self.dir.join("config"))
            .env("XDG_RUNTIME_DIR", self.dir.join("run"))
            .env("TMPDIR", self.dir.join("run"))
            .env("PATH", path)
            .env("DBUS_SYSTEM_BUS_ADDRESS", &self.system_address)
            .env("DBUS_SESSION_BUS_ADDRESS", &self.session_address)
            .env("WAYDROID_TRAY_IDLE_SECS", IDLE_SECS.to_string())
            .spawn()
            .unwrap();
        self.processes.push(tray);
        wait_for("the tray to register", QUICK, async || {
            self.mock().tray_item.is_some()
        })
        .await;
    }

    pub fn tray_running(&mut self) -> bool {
        self.processes
            .last_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
    }

    fn tray_item(&self) -> String {
        self.mock().tray_item.clone().unwrap()
    }

    async fn tray_call<B>(
        &self,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Message
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.session
            .call_method(Some(self.tray_item()), path, Some(interface), method, body)
            .await
            .unwrap()
    }

    pub async fn tray_property(&self, name: &str) -> String {
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

    pub async fn icon(&self) -> String {
        self.tray_property("IconName").await
    }

    /// A menu item's id and properties, by label.
    pub async fn menu_item(&self, label: &str) -> Option<(i32, HashMap<String, OwnedValue>)> {
        let reply = self
            .tray_call(
                "/MenuBar",
                "com.canonical.dbusmenu",
                "GetGroupProperties",
                &(Vec::<i32>::new(), vec!["label", "enabled"]),
            )
            .await;
        let items: Vec<(i32, HashMap<String, OwnedValue>)> = reply.body().deserialize().unwrap();
        items.into_iter().find(|(_, props)| {
            props
                .get("label")
                .and_then(|v| v.downcast_ref::<&str>().ok())
                == Some(label)
        })
    }

    pub async fn click(&self, label: &str) {
        let (id, _) = self
            .menu_item(label)
            .await
            .unwrap_or_else(|| panic!("no menu item {label:?}"));
        self.tray_call(
            "/MenuBar",
            "com.canonical.dbusmenu",
            "Event",
            &(id, "clicked", Value::from(0), 0u32),
        )
        .await;
    }

    /// ksni leaves `enabled` out when it's the default, true.
    pub async fn menu_enabled(&self, label: &str) -> bool {
        let (_, props) = self
            .menu_item(label)
            .await
            .unwrap_or_else(|| panic!("no menu item {label:?}"));
        props
            .get("enabled")
            .is_none_or(|v| v.downcast_ref::<bool>().unwrap())
    }

    pub async fn middle_click(&self) {
        self.tray_call(
            "/StatusNotifierItem",
            "org.kde.StatusNotifierItem",
            "SecondaryActivate",
            &(0i32, 0i32),
        )
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
        // Shim sessions run in their own process group, so they outlive the tray.
        let pids = fs::read_to_string(self.dir.join("pids")).unwrap_or_default();
        for pid in pids.lines() {
            let _ = Command::new("kill").arg(pid).status();
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

pub async fn wait_for(what: &str, within: Duration, mut check: impl AsyncFnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
