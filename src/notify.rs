//! Desktop notifications.

use std::collections::HashMap;

use zbus::Connection;
use zbus::zvariant::Value;

/// Autostarted, the tray has no terminal, so failures go to a notification.
pub async fn failure(session: &Connection, summary: &str, body: &str) {
    let hints: HashMap<&str, Value> = HashMap::new();
    let reply = session
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &("Waydroid", 0u32, "waydroid", summary, body, Vec::<&str>::new(), hints, -1i32),
        )
        .await;
    if let Err(err) = reply {
        eprintln!("{summary}: {body} (notification failed: {err})");
    }
}
