//! `--version` and `--help`.

use std::process::{Command, Output};

/// No HOME and no bus addresses, so anything that answers here answered before
/// the tray reached its setup, which is the point: these have to work while a
/// tray is already running.
fn run(arg: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_waydroid-tray"))
        .arg(arg)
        .env_remove("HOME")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("DBUS_SYSTEM_BUS_ADDRESS")
        .output()
        .unwrap()
}

fn stdout(arg: &str) -> String {
    let output = run(arg);
    assert!(output.status.success(), "{arg} exited {}", output.status);
    String::from_utf8(output.stdout).unwrap()
}

#[tokio::test]
async fn version_flag_prints_the_package_version() {
    let expected = format!("waydroid-tray {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(stdout("--version"), expected);
    assert_eq!(stdout("-V"), expected);
}

#[tokio::test]
async fn help_flag_lists_both_flags() {
    let help = stdout("--help");
    assert!(help.contains("--version"), "{help}");
    assert!(help.contains("--help"), "{help}");
    assert_eq!(stdout("-h"), help);
}
