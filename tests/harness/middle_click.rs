//! Middle click toggles the session (#8).

use crate::harness::{Harness, QUICK, wait_for};

#[tokio::test]
async fn middle_click_starts_a_stopped_session() {
    let mut h = Harness::new().await;
    h.start_tray().await;
    h.middle_click().await;
    wait_for("session start", QUICK, async || {
        h.waydroid_log() == ["session start"]
    })
    .await;
}

#[tokio::test]
async fn middle_click_stops_a_running_session() {
    let mut h = Harness::new().await;
    h.start_session("RUNNING").await;
    h.start_tray().await;
    h.middle_click().await;
    wait_for("session stop", QUICK, async || {
        h.waydroid_log() == ["session stop"]
    })
    .await;
}
