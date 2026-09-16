//! Waydroid's D-Bus services, queried without ever activating them.

use std::collections::HashMap;

use zbus::fdo::{DBusProxy, NameOwnerChanged};
use zbus::names::{BusName, OwnedUniqueName};
use zbus::proxy::MethodFlags;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy};

use crate::state::State;

pub const CONTAINER_NAME: &str = "id.waydro.Container";
pub const SESSION_NAME: &str = "id.waydro.Session";
const OBJECT_PATH: &str = "/ContainerManager";
const INTERFACE: &str = "id.waydro.ContainerManager";
/// The root service the container manager runs in. Stopping a session leaves
/// it up, holding its memory, so the tray offers a way to stop it too.
const CONTAINER_UNIT: &str = "waydroid-container.service";
const SYSTEMD_NAME: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";

/// The container manager's unique bus name, if it's running. Asking the bus
/// who owns a name never activates anything.
async fn container_owner(system: &Connection) -> Option<OwnedUniqueName> {
    let dbus = DBusProxy::new(system).await.ok()?;
    let name = BusName::try_from(CONTAINER_NAME).expect("valid bus name");
    dbus.get_name_owner(name).await.ok()
}

/// Reads the session state without ever starting the container service.
pub async fn read_state(system: &Connection) -> State {
    // Only talk to the container manager if it already owns its name, and
    // then via its unique name. Addressing the well-known name directly (like
    // `waydroid status` does) triggers D-Bus activation, which would start
    // waydroid-container.service on every poll.
    let Some(owner) = container_owner(system).await else {
        return State::Stopped;
    };
    let Ok(reply) = system
        .call_method(Some(owner), OBJECT_PATH, Some(INTERFACE), "GetSession", &())
        .await
    else {
        return State::Stopped;
    };
    let session: HashMap<String, String> = reply.body().deserialize().unwrap_or_default();
    match session.get("state").map(String::as_str) {
        None => State::Stopped,
        Some("STOPPED") => State::Stuck,
        Some("FROZEN") => State::Frozen,
        Some(_) => State::Running,
    }
}

/// Calls a container manager method that takes no arguments, e.g. `Freeze`.
/// Does nothing if the container manager isn't running.
pub async fn call(system: &Connection, method: &str) -> zbus::Result<()> {
    if let Some(owner) = container_owner(system).await {
        system
            .call_method(Some(owner), OBJECT_PATH, Some(INTERFACE), method, &())
            .await?;
    }
    Ok(())
}

/// Asks systemd to stop the container service, which runs as root. Polkit
/// checks the call, and `AllowInteractiveAuth` is what lets the desktop's
/// authentication agent prompt for it: without the flag polkit refuses on the
/// spot with "Interactive authentication required".
pub async fn stop_container_service(system: &Connection) -> zbus::Result<()> {
    let systemd = Proxy::new(system, SYSTEMD_NAME, SYSTEMD_PATH, SYSTEMD_MANAGER).await?;
    // "replace" is systemd's usual mode: take over from any queued job for the unit.
    let _job: Option<OwnedObjectPath> = systemd
        .call_with_flags(
            "StopUnit",
            MethodFlags::AllowInteractiveAuth.into(),
            &(CONTAINER_UNIT, "replace"),
        )
        .await?;
    Ok(())
}

pub fn gained_owner(change: &NameOwnerChanged) -> bool {
    change.args().is_ok_and(|args| args.new_owner.is_some())
}
