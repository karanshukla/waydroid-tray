//! Session state follows NameOwnerChanged (#2).

use std::time::Duration;

use crate::harness::{Harness, QUICK, SESSION_NAME, wait_for};

#[tokio::test]
async fn session_start_is_picked_up_without_polling() {
    let mut h = Harness::new().await;
    h.start_tray().await;
    assert_eq!(h.icon().await, "waydroid-tray-stopped-symbolic");
    h.start_session("RUNNING").await;
    wait_for("Running", QUICK, async || h.icon().await == "waydroid-tray-running-symbolic").await;
}

#[tokio::test]
async fn session_stop_is_immediate() {
    let mut h = Harness::new().await;
    h.start_session("RUNNING").await;
    h.start_tray().await;
    assert_eq!(h.icon().await, "waydroid-tray-running-symbolic");
    h.stop_session().await;
    wait_for("Stopped", QUICK, async || h.icon().await == "waydroid-tray-stopped-symbolic").await;
}

#[tokio::test]
async fn starting_session_can_be_stopped() {
    let mut h = Harness::new().await;
    h.start_container().await;
    h.start_tray().await;
    // The session manager is up, but the container has no session yet.
    h.session.request_name(SESSION_NAME).await.unwrap();
    wait_for("Starting", QUICK, async || h.menu_item("Waydroid: Starting...").await.is_some()).await;
    assert!(!h.menu_enabled("Start session").await);
    assert!(h.menu_enabled("Stop session").await);
    h.middle_click().await;
    wait_for("session stop", QUICK, async || h.waydroid_log() == ["session stop"]).await;
}

#[tokio::test]
async fn failed_stop_leaves_it_stuck() {
    let mut h = Harness::new().await;
    h.start_session("RUNNING").await;
    h.start_tray().await;
    // The stop failed partway, so the container keeps a stale session.
    h.mock().session_state = Some("STOPPED");
    h.session.release_name(SESSION_NAME).await.unwrap();
    let stuck = "Waydroid: Stuck (stop the session to reset)";
    wait_for("Stuck", QUICK, async || h.menu_item(stuck).await.is_some()).await;
    assert!(!h.menu_enabled("Start session").await);
    assert!(h.menu_enabled("Stop session").await);
    h.middle_click().await;
    wait_for("session stop", QUICK, async || h.waydroid_log() == ["session stop"]).await;
}

#[tokio::test]
async fn frozen_session_shows_frozen() {
    let mut h = Harness::new().await;
    h.start_session("FROZEN").await;
    h.start_tray().await;
    assert_eq!(h.icon().await, "waydroid-tray-frozen-symbolic");
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
