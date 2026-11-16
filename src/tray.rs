//! The tray icon and its menu.

use std::sync::Arc;

use ksni::ToolTip;
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
use tokio::sync::Notify;
use zbus::Connection;

use crate::apps::{AppEntry, read_icon};
use crate::config::Settings;
use crate::state::State;
use crate::{bus, cli, notify};

/// What the tray shows. The main loop redraws it only when this changes.
#[derive(Clone, PartialEq, Debug)]
pub struct Snapshot {
    pub state: State,
    /// The container service can be up with no session.
    pub container_up: bool,
    pub apps: Vec<AppEntry>,
}

pub struct WaydroidTray {
    state: State,
    container_up: bool,
    apps: Vec<AppItem>,
    icon_theme_path: String,
    settings: Settings,
    system: Connection,
    session: Connection,
    /// Notified after every menu action, so the main loop re-reads the state
    /// sooner.
    poke: Arc<Notify>,
}

struct AppItem {
    entry: AppEntry,
    png: Vec<u8>,
}

impl WaydroidTray {
    pub fn new(
        snapshot: Snapshot,
        icon_theme_path: String,
        settings: Settings,
        system: Connection,
        session: Connection,
        poke: Arc<Notify>,
    ) -> Self {
        let mut tray = Self {
            state: State::Stopped,
            container_up: false,
            apps: Vec::new(),
            icon_theme_path,
            settings,
            system,
            session,
            poke,
        };
        tray.show(snapshot);
        tray
    }

    pub fn show(&mut self, snapshot: Snapshot) {
        self.state = snapshot.state;
        self.container_up = snapshot.container_up;
        self.apps = snapshot
            .apps
            .into_iter()
            .map(|entry| AppItem {
                png: read_icon(&entry.icon),
                entry,
            })
            .collect();
    }

    fn run(&self, args: &[&str]) {
        cli::spawn_waydroid(args, self.session.clone());
        self.poke.notify_one();
    }

    fn call(&self, method: &'static str) {
        self.on_system_bus(format!("{method} failed"), move |system| async move {
            bus::call(&system, method).await
        });
    }

    fn stop_container_service(&self) {
        self.on_system_bus(
            "Stop container service failed".into(),
            |system| async move { bus::stop_container_service(&system).await },
        );
    }

    /// Runs `action` in the background, notifies `failure` if it fails, and
    /// pokes the main loop either way.
    fn on_system_bus<F, Fut>(&self, failure: String, action: F)
    where
        F: FnOnce(Connection) -> Fut + Send + 'static,
        Fut: Future<Output = zbus::Result<()>> + Send,
    {
        let (system, session, poke) =
            (self.system.clone(), self.session.clone(), self.poke.clone());
        tokio::spawn(async move {
            if let Err(err) = action(system).await {
                notify::failure(&session, &failure, &err.to_string()).await;
            }
            poke.notify_one();
        });
    }

    fn app_items(&self) -> Vec<MenuItem<Self>> {
        self.apps
            .iter()
            .map(|app| {
                let package = app.entry.package.clone();
                StandardItem {
                    label: app.entry.name.clone(),
                    icon_data: app.png.clone(),
                    activate: Box::new(move |tray: &mut Self| {
                        tray.run(&["app", "launch", &package])
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect()
    }
}

impl ksni::Tray for WaydroidTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        "Waydroid".into()
    }

    fn status(&self) -> ksni::Status {
        if self.settings.config.hide_when_stopped && self.state == State::Stopped {
            ksni::Status::Passive
        } else {
            ksni::Status::Active
        }
    }

    fn icon_name(&self) -> String {
        self.state.icon().into()
    }

    fn icon_theme_path(&self) -> String {
        self.icon_theme_path.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: format!("Waydroid: {}", self.state.label()),
            ..Default::default()
        }
    }

    /// Middle click toggles the session (pinned by
    /// `harness::middle_click::middle_click_starts_a_stopped_session`,
    /// `harness::middle_click::middle_click_stops_a_running_session`).
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        if self.state == State::Stopped {
            self.run(&["session", "start"]);
        } else {
            self.run(&["session", "stop"]);
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let stopped = self.state == State::Stopped;
        let active = matches!(self.state, State::Running | State::Frozen);
        let freeze_method = if self.state == State::Frozen {
            "Unfreeze"
        } else {
            "Freeze"
        };
        let apps = self.app_items();

        vec![
            StandardItem {
                label: format!("Waydroid: {}", self.state.label()),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Start session".into(),
                enabled: stopped,
                activate: Box::new(|tray: &mut Self| tray.run(&["session", "start"])),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Stop session".into(),
                enabled: !stopped,
                activate: Box::new(|tray: &mut Self| tray.run(&["session", "stop"])),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Stop container service".into(),
                enabled: stopped && self.container_up,
                activate: Box::new(|tray: &mut Self| tray.stop_container_service()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: freeze_method.into(),
                enabled: active,
                activate: Box::new(move |tray: &mut Self| tray.call(freeze_method)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Show full UI".into(),
                activate: Box::new(|tray: &mut Self| tray.run(&["show-full-ui"])),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Apps".into(),
                enabled: !apps.is_empty(),
                submenu: apps,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Start session at login".into(),
                checked: self.settings.config.start_at_login,
                activate: Box::new(|tray: &mut Self| {
                    tray.settings.toggle(|config| &mut config.start_at_login);
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Stop session after 30 min idle".into(),
                checked: self.settings.config.auto_stop_when_idle,
                activate: Box::new(|tray: &mut Self| {
                    tray.settings
                        .toggle(|config| &mut config.auto_stop_when_idle);
                    tray.poke.notify_one();
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Hide icon while stopped".into(),
                checked: self.settings.config.hide_when_stopped,
                activate: Box::new(|tray: &mut Self| {
                    tray.settings.toggle(|config| &mut config.hide_when_stopped);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit tray".into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}
