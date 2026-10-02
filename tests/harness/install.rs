//! `--install` and `--uninstall`.

use std::fs;

use crate::harness::Harness;

const FILES: [&str; 7] = [
    ".local/share/icons/hicolor/scalable/status/waydroid-tray-running-symbolic.svg",
    ".local/share/icons/hicolor/scalable/status/waydroid-tray-frozen-symbolic.svg",
    ".local/share/icons/hicolor/scalable/status/waydroid-tray-stopped-symbolic.svg",
    ".local/share/icons/hicolor/scalable/apps/waydroid-tray.svg",
    ".config/systemd/user/waydroid-tray.service",
    ".local/share/applications/waydroid-tray.desktop",
    ".config/autostart/waydroid-tray.desktop",
];

fn take_calls(harness: &Harness) -> Vec<String> {
    std::mem::take(&mut harness.mock().user_systemd_calls)
}

#[tokio::test]
async fn install_writes_the_setup_and_uninstall_removes_it() {
    let harness = Harness::new().await;
    let home = harness.dir.join("home");
    let old_icon = home.join(".local/share/icons/hicolor/scalable/status/waydroid-tray-old.svg");
    fs::create_dir_all(old_icon.parent().unwrap()).unwrap();
    fs::write(&old_icon, "").unwrap();

    harness.run_setup("--install", "wayland-1").await;
    for file in FILES {
        assert!(home.join(file).is_file(), "{file} missing");
    }
    assert!(!old_icon.exists());
    let unit = fs::read_to_string(home.join(FILES[4])).unwrap();
    let exec = format!("ExecStart=\"{}\"\n", env!("CARGO_BIN_EXE_waydroid-tray"));
    assert!(unit.contains(&exec), "{unit}");
    assert_eq!(
        take_calls(&harness),
        [
            "Reload",
            "EnableUnitFiles waydroid-tray.service",
            "StopUnit waydroid-tray.service",
            "SetEnvironment WAYLAND_DISPLAY=wayland-1",
            "StartUnit waydroid-tray.service",
        ]
    );

    harness.run_setup("--uninstall", "wayland-1").await;
    for file in FILES {
        assert!(!home.join(file).exists(), "{file} left behind");
    }
    assert_eq!(
        take_calls(&harness),
        [
            "DisableUnitFiles waydroid-tray.service",
            "StopUnit waydroid-tray.service",
            "Reload",
        ]
    );
}

#[tokio::test]
async fn install_outside_a_desktop_session_leaves_the_tray_for_next_login() {
    let harness = Harness::new().await;
    harness.run_setup("--install", "").await;
    assert_eq!(
        take_calls(&harness),
        [
            "Reload",
            "EnableUnitFiles waydroid-tray.service",
            "StopUnit waydroid-tray.service",
        ]
    );
}

#[tokio::test]
async fn install_starts_the_tray_in_a_session_without_wayland_display() {
    let harness = Harness::new().await;
    harness.mock().graphical_session = true;
    harness.run_setup("--install", "").await;
    assert_eq!(
        take_calls(&harness),
        [
            "Reload",
            "EnableUnitFiles waydroid-tray.service",
            "StopUnit waydroid-tray.service",
            "StartUnit waydroid-tray.service",
        ]
    );
}
