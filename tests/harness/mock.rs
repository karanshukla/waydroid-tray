//! Stand-ins for the D-Bus services the tray talks to.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::interface;
use zbus::message::{Flags, Header};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

#[derive(Default)]
pub struct Mock {
    /// What `GetSession` reports as `state`; `None` means no session.
    pub session_state: Option<&'static str>,
    pub get_session_calls: usize,
    pub container_calls: Vec<&'static str>,
    /// Make `Freeze`/`Unfreeze` fail like an uninitialized Waydroid.
    pub container_fails: bool,
    /// `(unit, mode)` for every `StopUnit` systemd was asked for.
    pub stopped_units: Vec<(String, String)>,
    /// Whether the last `StopUnit` let polkit prompt.
    pub interactive_stop: bool,
    /// Refuse `StopUnit` the way polkit does when the prompt is dismissed.
    pub systemd_denies: bool,
    pub notifications: Vec<(String, String)>,
    pub tray_item: Option<String>,
}

pub type Shared = Arc<Mutex<Mock>>;

pub struct ContainerManager(pub Shared);

impl ContainerManager {
    fn record(&self, method: &'static str) -> zbus::fdo::Result<()> {
        let mut mock = self.0.lock().unwrap();
        mock.container_calls.push(method);
        match mock.container_fails {
            true => Err(zbus::fdo::Error::Failed("WayDroid is not initialized".into())),
            false => Ok(()),
        }
    }
}

#[interface(name = "id.waydro.ContainerManager")]
impl ContainerManager {
    fn get_session(&self) -> HashMap<String, String> {
        let mut mock = self.0.lock().unwrap();
        mock.get_session_calls += 1;
        mock.session_state.map(|state| HashMap::from([("state".into(), state.into())])).unwrap_or_default()
    }

    fn freeze(&self) -> zbus::fdo::Result<()> {
        self.record("Freeze")
    }

    fn unfreeze(&self) -> zbus::fdo::Result<()> {
        self.record("Unfreeze")
    }
}

/// Enough of systemd's manager to see what the tray asks it to stop.
pub struct Systemd(pub Shared);

#[interface(name = "org.freedesktop.systemd1.Manager")]
impl Systemd {
    fn stop_unit(
        &self,
        #[zbus(header)] header: Header<'_>,
        name: String,
        mode: String,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let mut mock = self.0.lock().unwrap();
        mock.interactive_stop = header.primary().flags().contains(Flags::AllowInteractiveAuth);
        mock.stopped_units.push((name, mode));
        if mock.systemd_denies {
            return Err(zbus::fdo::Error::AccessDenied("Interactive authentication required".into()));
        }
        Ok(OwnedObjectPath::try_from("/org/freedesktop/systemd1/job/1").unwrap())
    }
}

pub struct Notifications(pub Shared);

#[interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        _app_name: String,
        _replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        _hints: HashMap<String, OwnedValue>,
        _expire_timeout: i32,
    ) -> u32 {
        let mut mock = self.0.lock().unwrap();
        mock.notifications.push((summary, body));
        mock.notifications.len() as u32
    }
}

/// Stands in for Plasma, which the tray has to register with to start.
pub struct Watcher(pub Shared);

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: String) {
        self.0.lock().unwrap().tray_item = Some(service);
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}
