//! Freeze and unfreeze (#5).

use crate::harness::{Harness, QUICK, wait_for};

#[tokio::test]
async fn freeze_when_running() {
    let mut h = Harness::new().await;
    h.start_session("RUNNING").await;
    h.start_tray().await;
    h.click("Freeze").await;
    wait_for("Freeze", QUICK, async || {
        h.mock().container_calls == ["Freeze"]
    })
    .await;
}

#[tokio::test]
async fn unfreeze_when_frozen() {
    let mut h = Harness::new().await;
    h.start_session("FROZEN").await;
    h.start_tray().await;
    h.click("Unfreeze").await;
    wait_for("Unfreeze", QUICK, async || {
        h.mock().container_calls == ["Unfreeze"]
    })
    .await;
}

#[tokio::test]
async fn freeze_error_is_notified() {
    let mut h = Harness::new().await;
    h.start_session("RUNNING").await;
    h.mock().container_fails = true;
    h.start_tray().await;
    h.click("Freeze").await;
    wait_for("a notification", QUICK, async || {
        !h.mock().notifications.is_empty()
    })
    .await;
    let (summary, body) = h.mock().notifications[0].clone();
    assert_eq!(summary, "Freeze failed");
    assert!(body.contains("not initialized"), "{body}");
    assert!(h.tray_running());
}
