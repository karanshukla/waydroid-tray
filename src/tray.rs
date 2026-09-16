//! The tray icon and its menu.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use ksni::ToolTip;
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
use tokio::sync::Notify;
use zbus::Connection;

use crate::apps::AppEntry;
use crate::config::Config;
use crate::state::State;
use crate::{bus, cli, notify};

pub struct WaydroidTray {
    pub state: State,
    /// Whether the container service is up, which it can be with no session.
    pub container_up: bool,
    apps: Vec<(AppEntry, Vec<u8>)>,
    icon_theme_path: String,
    config: Config,
    config_path: PathBuf,
    system: Connection,
    session: Connection,
    poke: Arc<Notify>,
}

impl WaydroidTray {
    /// `poke` is notified after every menu action, so the caller can refresh
    /// the state sooner.
    pub fn new(
        state: State,
        icon_theme_path: String,
        config: Config,
        config_path: PathBuf,
        system: Connection,
        session: Connection,
        poke: Arc<Notify>,
    ) -> Self {
        Self {
            state,
            container_up: false,
            apps: Vec::new(),
            icon_theme_path,
            config,
            config_path,
            system,
            session,
            poke,
        }
    }

    fn run(&self, args: &[&str]) {
        cli::spawn_waydroid(args, self.session.clone());
        self.poke.notify_one();
    }

    fn call(&self, method: &'static str) {
        let (system, session, poke) = (self.system.clone(), self.session.clone(), self.poke.clone());
        tokio::spawn(async move {
            if let Err(err) = bus::call(&system, method).await {
                notify::failure(&session, &format!("{method} failed"), &err.to_string()).await;
            }
            poke.notify_one();
        });
    }

    fn stop_container_service(&self) {
        let (system, session, poke) = (self.system.clone(), self.session.clone(), self.poke.clone());
        tokio::spawn(async move {
            if let Err(err) = bus::stop_container_service(&system).await {
                notify::failure(&session, "Stop container service failed", &err.to_string()).await;
            }
            poke.notify_one();
        });
    }

    fn save_config(&self) {
        if let Err(err) = self.config.save(&self.config_path) {
            eprintln!("failed to save {}: {err}", self.config_path.display());
        }
    }

    pub fn set_apps(&mut self, apps: Vec<AppEntry>) {
        self.apps = apps
            .into_iter()
            .map(|app| {
                let png = fs::read(&app.icon).unwrap_or_default();
                (app, png)
            })
            .collect();
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
        // Plasma tucks passive items away in the hidden icons.
        if self.config.hide_when_stopped && self.state == State::Stopped {
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

    /// Middle click toggles the session.
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
        // Also the container manager method to call.
        let freeze = if self.state == State::Frozen { "Unfreeze" } else { "Freeze" };
        let apps: Vec<MenuItem<Self>> = self
            .apps
            .iter()
            .map(|(app, png)| {
                let package = app.package.clone();
                StandardItem {
                    label: app.name.clone(),
                    icon_data: png.clone(),
                    activate: Box::new(move |tray: &mut Self| {
                        tray.run(&["app", "launch", &package])
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();

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
                // Only once the session is down: the service is what runs it.
                enabled: stopped && self.container_up,
                activate: Box::new(|tray: &mut Self| tray.stop_container_service()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: freeze.into(),
                enabled: active,
                activate: Box::new(move |tray: &mut Self| tray.call(freeze)),
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
                checked: self.config.start_at_login,
                activate: Box::new(|tray: &mut Self| {
                    tray.config.start_at_login ^= true;
                    tray.save_config();
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Hide icon while stopped".into(),
                checked: self.config.hide_when_stopped,
                activate: Box::new(|tray: &mut Self| {
                    tray.config.hide_when_stopped ^= true;
                    tray.save_config();
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
