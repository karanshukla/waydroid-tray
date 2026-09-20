//! Stop the container service (#7).

use crate::harness::{Harness, QUICK, wait_for};

#[tokio::test]
async fn stop_container_service_is_offered_once_the_session_is_down() {
    let mut h = Harness::new().await;
    h.start_tray().await;
    // Nothing to stop: the service isn't running either.
    assert!(!h.menu_enabled("Stop container service").await);
    h.start_session("RUNNING").await;
    wait_for("Running", QUICK, async || {
        h.icon().await == "waydroid-tray-running-symbolic"
    })
    .await;
    assert!(!h.menu_enabled("Stop container service").await);
    // The service outlives the session, which is what the item is for.
    h.stop_session().await;
    wait_for("the item to enable", QUICK, async || {
        h.menu_enabled("Stop container service").await
    })
    .await;
}

#[tokio::test]
async fn stop_container_service_stops_the_unit_through_systemd() {
    let mut h = Harness::new().await;
    h.start_container().await;
    h.start_tray().await;
    h.click("Stop container service").await;
    wait_for("StopUnit", QUICK, async || {
        !h.mock().stopped_units.is_empty()
    })
    .await;
    assert_eq!(
        h.mock().stopped_units,
        [("waydroid-container.service".into(), "replace".into())]
    );
    // Without this flag polkit refuses outright instead of letting the agent prompt.
    assert!(h.mock().interactive_stop);
}

#[tokio::test]
async fn refused_container_service_stop_is_notified() {
    let mut h = Harness::new().await;
    h.start_container().await;
    h.mock().systemd_denies = true;
    h.start_tray().await;
    h.click("Stop container service").await;
    wait_for("a notification", QUICK, async || {
        !h.mock().notifications.is_empty()
    })
    .await;
    let (summary, body) = h.mock().notifications[0].clone();
    assert_eq!(summary, "Stop container service failed");
    assert!(body.contains("authentication"), "{body}");
    assert!(h.tray_running());
}
