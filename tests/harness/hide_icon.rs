//! Hide the icon while stopped (#3).

use std::fs;

use crate::harness::{Harness, QUICK, wait_for};

#[tokio::test]
async fn hidden_while_stopped_when_enabled() {
    let mut h = Harness::new().await;
    h.write_config("hide_when_stopped=true\n");
    h.start_tray().await;
    assert_eq!(h.tray_property("Status").await, "Passive");
    h.start_session("RUNNING").await;
    wait_for("Active", QUICK, async || h.tray_property("Status").await == "Active").await;
}

#[tokio::test]
async fn hide_toggle_applies_and_persists() {
    let mut h = Harness::new().await;
    h.start_tray().await;
    assert_eq!(h.tray_property("Status").await, "Active");
    h.click("Hide icon while stopped").await;
    assert_eq!(h.tray_property("Status").await, "Passive");
    let config = fs::read_to_string(h.dir.join("config/waydroid-tray/config")).unwrap();
    assert!(config.contains("hide_when_stopped=true"), "{config}");
}
