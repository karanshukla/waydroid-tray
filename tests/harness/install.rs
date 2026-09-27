//! `--install` and `--uninstall`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const SYSTEMCTL: &str = "#!/bin/sh\necho \"$*\" >> \"${0%/*}/../systemctl.log\"\n";

const FILES: [&str; 7] = [
    ".local/share/icons/hicolor/scalable/status/waydroid-tray-running-symbolic.svg",
    ".local/share/icons/hicolor/scalable/status/waydroid-tray-frozen-symbolic.svg",
    ".local/share/icons/hicolor/scalable/status/waydroid-tray-stopped-symbolic.svg",
    ".local/share/icons/hicolor/scalable/apps/waydroid-tray.svg",
    ".config/systemd/user/waydroid-tray.service",
    ".local/share/applications/waydroid-tray.desktop",
    ".config/autostart/waydroid-tray.desktop",
];

/// PATH holds only a `systemctl` that logs its arguments. With no `pgrep` on
/// it, the other tests' trays are safe from the stop.
fn run(dir: &Path, arg: &str) -> Vec<String> {
    let status = Command::new(env!("CARGO_BIN_EXE_waydroid-tray"))
        .arg(arg)
        .env("HOME", dir.join("home"))
        .env("PATH", dir.join("bin"))
        .env_remove("WAYLAND_DISPLAY")
        .status()
        .unwrap();
    assert!(status.success(), "{arg} exited {status}");
    let log = fs::read_to_string(dir.join("systemctl.log")).unwrap_or_default();
    fs::remove_file(dir.join("systemctl.log")).unwrap();
    log.lines().map(String::from).collect()
}

#[test]
fn install_writes_the_setup_and_uninstall_removes_it() {
    let dir = std::env::temp_dir().join(format!("waydroid-tray-install-{}", std::process::id()));
    let home = dir.join("home");
    let shim = dir.join("bin/systemctl");
    fs::create_dir_all(shim.parent().unwrap()).unwrap();
    fs::write(&shim, SYSTEMCTL).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    let old_icon = home.join(".local/share/icons/hicolor/scalable/status/waydroid-tray-old.svg");
    fs::create_dir_all(old_icon.parent().unwrap()).unwrap();
    fs::write(&old_icon, "").unwrap();

    let calls = run(&dir, "--install");
    for file in FILES {
        assert!(home.join(file).is_file(), "{file} missing");
    }
    assert!(!old_icon.exists());
    let unit = fs::read_to_string(home.join(FILES[4])).unwrap();
    let exec = format!("ExecStart={}\n", env!("CARGO_BIN_EXE_waydroid-tray"));
    assert!(unit.contains(&exec), "{unit}");
    assert_eq!(
        calls,
        [
            "--user daemon-reload",
            "--user enable waydroid-tray",
            "--user stop waydroid-tray",
            "--user is-active graphical-session.target",
            "--user start waydroid-tray",
        ]
    );

    let calls = run(&dir, "--uninstall");
    for file in FILES {
        assert!(!home.join(file).exists(), "{file} left behind");
    }
    assert_eq!(
        calls,
        [
            "--user disable waydroid-tray",
            "--user stop waydroid-tray",
            "--user daemon-reload",
        ]
    );
    fs::remove_dir_all(&dir).unwrap();
}
