//! Stop the session once it has been idle for the timeout.

use std::fs;
use std::time::Duration;

use crate::harness::{Harness, IDLE_SECS, QUICK, wait_for};

/// The mock stays frozen, so the tray keeps retrying; only the first command
/// is the one under test.
fn stopped_once(h: &Harness) -> bool {
    h.waydroid_log()
        .first()
        .is_some_and(|line| line == "session stop")
}

#[tokio::test]
async fn frozen_session_stops_once_idle() {
    let mut h = Harness::new().await;
    h.write_config("auto_stop_when_idle=true\n");
    h.start_session("FROZEN").await;
    h.start_tray().await;
    wait_for("session stop", QUICK, async || stopped_once(&h)).await;
}

#[tokio::test]
async fn running_session_is_never_idle_stopped() {
    let mut h = Harness::new().await;
    h.write_config("auto_stop_when_idle=true\n");
    h.start_session("RUNNING").await;
    h.start_tray().await;
    tokio::time::sleep(Duration::from_secs(3 * IDLE_SECS)).await;
    assert!(h.waydroid_log().is_empty(), "{:?}", h.waydroid_log());
}

#[tokio::test]
async fn frozen_session_stays_up_while_the_toggle_is_off() {
    let mut h = Harness::new().await;
    h.start_session("FROZEN").await;
    h.start_tray().await;
    tokio::time::sleep(Duration::from_secs(3 * IDLE_SECS)).await;
    assert!(h.waydroid_log().is_empty(), "{:?}", h.waydroid_log());
}

#[tokio::test]
async fn idle_toggle_arms_the_timer_and_persists() {
    let mut h = Harness::new().await;
    h.start_session("FROZEN").await;
    h.start_tray().await;
    h.click("Stop session after 30 min idle").await;
    let config = fs::read_to_string(h.dir.join("config/waydroid-tray/config")).unwrap();
    assert!(config.contains("auto_stop_when_idle=true"), "{config}");
    // Arming restarts the clock, so this waits out the settle plus the timeout.
    wait_for("session stop", Duration::from_secs(6), async || {
        stopped_once(&h)
    })
    .await;
}
