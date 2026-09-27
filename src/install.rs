//! `--install` and `--uninstall`: the icons, systemd user unit, and app menu
//! and autostart entries that `cargo install` can't put in place.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy};

use crate::bus::{SYSTEMD_MANAGER, SYSTEMD_NAME, SYSTEMD_PATH};

const UNIT: &str = include_str!("../waydroid-tray.service");
const DESKTOP: &str = include_str!("../waydroid-tray.desktop");
const APP_ICON: &str = include_str!("../icons/waydroid-tray.svg");
const STATUS_ICONS: [(&str, &str); 3] = [
    (
        "waydroid-tray-running-symbolic.svg",
        include_str!("../icons/waydroid-tray-running-symbolic.svg"),
    ),
    (
        "waydroid-tray-frozen-symbolic.svg",
        include_str!("../icons/waydroid-tray-frozen-symbolic.svg"),
    ),
    (
        "waydroid-tray-stopped-symbolic.svg",
        include_str!("../icons/waydroid-tray-stopped-symbolic.svg"),
    ),
];
/// Where `waydroid-tray.service` expects the binary. `--install` points it at
/// this one instead.
const DEFAULT_EXEC: &str = "%h/.local/bin/waydroid-tray";
const UNIT_NAME: &str = "waydroid-tray.service";
const SYSTEMD_UNIT: &str = "org.freedesktop.systemd1.Unit";

struct Paths {
    status_icons: PathBuf,
    app_icon: PathBuf,
    unit: PathBuf,
    launcher: PathBuf,
    /// The unit starts with graphical-session.target, which many desktops
    /// never reach. Their autostart runs the launcher, which starts the unit.
    autostart: PathBuf,
}

impl Paths {
    fn new(home: &Path) -> Self {
        Paths {
            status_icons: home.join(".local/share/icons/hicolor/scalable/status"),
            app_icon: home.join(".local/share/icons/hicolor/scalable/apps/waydroid-tray.svg"),
            unit: home.join(".config/systemd/user/waydroid-tray.service"),
            launcher: home.join(".local/share/applications/waydroid-tray.desktop"),
            autostart: home.join(".config/autostart/waydroid-tray.desktop"),
        }
    }
}

pub async fn install(home: &Path) -> io::Result<()> {
    let paths = Paths::new(home);
    let exe = std::env::current_exe()?;
    remove_status_icons(&paths.status_icons)?;
    for (name, svg) in STATUS_ICONS {
        write(&paths.status_icons.join(name), svg)?;
    }
    write(&paths.app_icon, APP_ICON)?;
    write(
        &paths.unit,
        &UNIT.replace(DEFAULT_EXEC, &exe.to_string_lossy()),
    )?;
    write(&paths.launcher, DESKTOP)?;
    write(&paths.autostart, DESKTOP)?;

    let systemd = UserSystemd::connect().await?;
    systemd.call("Reload", &()).await?;
    systemd
        .call("EnableUnitFiles", &(&[UNIT_NAME][..], false, false))
        .await?;
    stop_tray(Some(&systemd)).await;
    let wayland = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    // Outside a desktop session (e.g. over ssh) there's no tray to show it in.
    if wayland.is_empty() && !systemd.is_active("graphical-session.target").await {
        println!("Installed. The tray starts with your next desktop session.");
        return Ok(());
    }
    // Waydroid needs it, and not every desktop passes it on to systemd.
    if !wayland.is_empty() {
        let assignment = format!("WAYLAND_DISPLAY={wayland}");
        systemd
            .call("SetEnvironment", &(&[assignment][..],))
            .await?;
    }
    systemd.call("StartUnit", &(UNIT_NAME, "replace")).await?;
    println!("Installed and started the tray.");
    Ok(())
}

/// Leaves the binary itself to whatever installed it.
pub async fn uninstall(home: &Path) -> io::Result<()> {
    let paths = Paths::new(home);
    let systemd = UserSystemd::connect().await.ok();
    if let Some(systemd) = &systemd {
        let _ = systemd
            .call("DisableUnitFiles", &(&[UNIT_NAME][..], false))
            .await;
    }
    stop_tray(systemd.as_ref()).await;
    remove_status_icons(&paths.status_icons)?;
    for path in [
        &paths.app_icon,
        &paths.unit,
        &paths.launcher,
        &paths.autostart,
    ] {
        remove(path)?;
    }
    if let Some(systemd) = &systemd {
        let _ = systemd.call("Reload", &()).await;
    }
    println!("Removed the waydroid-tray unit, icons and menu entries.");
    Ok(())
}

fn write(path: &Path, contents: &str) -> io::Result<()> {
    fs::create_dir_all(path.parent().expect("an absolute path"))
        .and_then(|()| fs::write(path, contents))
        .map_err(|err| io::Error::new(err.kind(), format!("write {}: {err}", path.display())))
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(io::Error::new(
            err.kind(),
            format!("remove {}: {err}", path.display()),
        )),
        _ => Ok(()),
    }
}

/// Including any an older version named differently.
fn remove_status_icons(dir: &Path) -> io::Result<()> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with("waydroid-tray-") && name.ends_with(".svg") {
            remove(&path)?;
        }
    }
    Ok(())
}

/// The user's systemd manager, over the session bus. `systemctl` can't be
/// relied on for this: toolbox and distrobox images may not ship it, and in a
/// container sharing the host's PID namespace it decides it's in a chroot and
/// skips the command without failing. Both share the host's session bus.
struct UserSystemd(Proxy<'static>);

impl UserSystemd {
    async fn connect() -> io::Result<Self> {
        let connect = async {
            let bus = Connection::session().await?;
            Proxy::new_owned(bus, SYSTEMD_NAME, SYSTEMD_PATH, SYSTEMD_MANAGER).await
        };
        connect
            .await
            .map(UserSystemd)
            .map_err(|err| io::Error::other(format!("reach the user's systemd: {err}")))
    }

    async fn call<B>(&self, method: &str, body: &B) -> io::Result<()>
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.0
            .call_method(method, body)
            .await
            .map(drop)
            .map_err(|err| io::Error::other(format!("systemd {method}: {err}")))
    }

    async fn is_active(&self, unit: &str) -> bool {
        let Ok(reply) = self.0.call_method("GetUnit", &(unit,)).await else {
            return false;
        };
        let Ok(path) = reply.body().deserialize::<OwnedObjectPath>() else {
            return false;
        };
        let Ok(unit) = Proxy::new(self.0.connection(), SYSTEMD_NAME, path, SYSTEMD_UNIT).await
        else {
            return false;
        };
        unit.get_property::<String>("ActiveState")
            .await
            .is_ok_and(|state| state == "active")
    }
}

/// Stops the unit and any tray started outside it, e.g. by an older version,
/// and waits for them to exit and drop the lock. Any still running after five
/// seconds get SIGKILL.
async fn stop_tray(systemd: Option<&UserSystemd>) {
    if let Some(systemd) = systemd {
        let _ = systemd.call("StopUnit", &(UNIT_NAME, "replace")).await;
    }
    for _ in 0..50 {
        let others = other_trays();
        if others.is_empty() {
            return;
        }
        kill(&others, "-TERM");
        std::thread::sleep(Duration::from_millis(100));
    }
    kill(&other_trays(), "-KILL");
}

fn kill(pids: &[String], signal: &str) {
    if pids.is_empty() {
        return;
    }
    let _ = Command::new("kill")
        .arg(signal)
        .args(pids)
        .stderr(Stdio::null())
        .status();
}

/// This user's other `waydroid-tray` processes. Not this one, which has the
/// same name.
fn other_trays() -> Vec<String> {
    let Ok(uid) = fs::metadata("/proc/self").map(|proc| proc.uid()) else {
        return Vec::new();
    };
    let Ok(output) = Command::new("pgrep")
        .args(["-x", "-u", &uid.to_string(), "waydroid-tray"])
        .output()
    else {
        return Vec::new();
    };
    let me = std::process::id().to_string();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|pid| *pid != me)
        .map(String::from)
        .collect()
}
