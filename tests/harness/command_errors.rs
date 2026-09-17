//! Notify when a waydroid command fails (#6).

use std::time::Duration;

use crate::harness::{Harness, QUICK, wait_for};

#[tokio::test]
async fn failed_command_is_notified_with_stderr() {
    let mut h = Harness::new().await;
    h.fail_waydroid(1);
    h.start_tray().await;
    h.click("Show full UI").await;
    wait_for("a notification", QUICK, async || !h.mock().notifications.is_empty()).await;
    let notifications = h.mock().notifications.clone();
    assert_eq!(
        notifications,
        [("waydroid show-full-ui failed".into(), "shim: show-full-ui failed".into())]
    );
}

#[tokio::test]
async fn session_start_exiting_zero_early_is_notified() {
    // Waydroid exits 0 when the container isn't listening.
    let mut h = Harness::new().await;
    h.fail_waydroid(0);
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
async fn lasting_session_start_is_not_notified() {
    let mut h = Harness::new().await;
    h.start_tray().await;
    h.click("Start session").await;
    wait_for("the command", QUICK, async || h.waydroid_log() == ["session start"]).await;
    // Past the tray's 5s grace period.
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(h.mock().notifications.is_empty(), "{:?}", h.mock().notifications);
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
