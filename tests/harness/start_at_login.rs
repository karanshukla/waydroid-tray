//! Start the session at login (#4).

use std::time::Duration;

use crate::harness::{Harness, QUICK, wait_for};

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
