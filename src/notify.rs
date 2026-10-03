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
            &(
                "Waydroid",
                0u32,
                "waydroid-tray",
                summary,
                escape_markup(body).as_str(),
                Vec::<&str>::new(),
                hints,
                -1i32,
            ),
        )
        .await;
    if let Err(err) = reply {
        eprintln!("{summary}: {body} (notification failed: {err})");
    }
}

/// Servers may render the body as markup, and it's Waydroid's error output.
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_in_the_body_is_shown_as_text() {
        assert_eq!(
            escape_markup(r#"<a href="x">&amp;</a>"#),
            r#"&lt;a href="x"&gt;&amp;amp;&lt;/a&gt;"#
        );
    }
}
