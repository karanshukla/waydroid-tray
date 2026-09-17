//! Runs the real tray against private system and session buses, with mock
//! Waydroid, notification and tray host services, and a `waydroid` shim on
//! PATH. Needs `busd` on PATH (`cargo install busd`), nothing else.

mod harness;
mod mock;

mod command_errors;
mod container_service;
mod freeze;
mod hide_icon;
mod middle_click;
mod session_state;
mod start_at_login;
