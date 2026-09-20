//! Stop the session once it has been idle for the timeout.

use std::fs;
use std::time::{Duration, Instant};

use crate::harness::{Harness, IDLE_SECS, wait_for};

/// The mock never leaves FROZEN, so the tray keeps retrying; only the first
/// command is the one under test.
fn stopped_once(h: &Harness) -> bool {
    h.waydroid_log()
        .first()
        .is_some_and(|line| line == "session stop")
}

fn idle() -> Duration {
    Duration::from_secs(IDLE_SECS)
}

/// Long enough for a stop to land after the timeout, plus the tray's settle.
fn idle_budget() -> Duration {
    idle() * 3
}

#[tokio::test]
async fn frozen_session_stops_once_idle() {
    let mut h = Harness::new().await;
    h.write_config("auto_stop_when_idle=true\n");
    h.start_session("FROZEN").await;
    h.start_tray().await;
    wait_for("session stop", idle_budget(), async || stopped_once(&h)).await;
}

#[tokio::test]
async fn running_session_is_never_idle_stopped() {
    let mut h = Harness::new().await;
    h.write_config("auto_stop_when_idle=true\n");
    h.start_session("RUNNING").await;
    h.start_tray().await;
    tokio::time::sleep(idle_budget()).await;
    assert!(h.waydroid_log().is_empty(), "{:?}", h.waydroid_log());
}

#[tokio::test]
async fn frozen_session_stays_up_while_the_toggle_is_off() {
    let mut h = Harness::new().await;
    h.start_session("FROZEN").await;
    h.start_tray().await;
    tokio::time::sleep(idle_budget()).await;
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
    wait_for("session stop", idle_budget(), async || stopped_once(&h)).await;
}

/// The pair for `arming_the_toggle_restarts_the_clock`: a session that has
/// been frozen for longer than the timeout is still given a full timeout from
/// the moment the toggle goes on, not stopped straight away.
#[tokio::test]
async fn arming_the_toggle_restarts_the_clock() {
    let mut h = Harness::new().await;
    h.start_session("FROZEN").await;
    h.start_tray().await;
    tokio::time::sleep(idle() * 2).await;
    assert!(h.waydroid_log().is_empty(), "{:?}", h.waydroid_log());

    let armed_at = Instant::now();
    h.click("Stop session after 30 min idle").await;
    wait_for("session stop", idle_budget(), async || stopped_once(&h)).await;
    let waited = armed_at.elapsed();
    assert!(
        waited >= idle(),
        "stopped {waited:?} after arming, which is inside the {:?} timeout",
        idle()
    );
}

#[tokio::test]
async fn a_stop_that_leaves_it_frozen_waits_out_the_timeout_again() {
    let mut h = Harness::new().await;
    h.write_config("auto_stop_when_idle=true\n");
    h.start_session("FROZEN").await;
    h.start_tray().await;
    wait_for("session stop", idle_budget(), async || stopped_once(&h)).await;
    assert_eq!(h.waydroid_log().len(), 1, "{:?}", h.waydroid_log());
    wait_for("a retry", idle_budget(), async || {
        h.waydroid_log().len() >= 2
    })
    .await;
}
