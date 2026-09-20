# waydroid-tray

A small system tray icon for [Waydroid](https://waydro.id). Waydroid doesn't ship one, so once it's running in the background there's no way to see its state or stop it without a terminal.

The icon is a monochrome Android head that follows your panel's colours, like the symbolic icons next to it:

| Icon | State |
|---|---|
| Outline | Session stopped |
| Filled, eyes closed | Frozen (no Android windows open, 0% CPU but still holding RAM) |
| Filled, eyes open | Running |

The outline also covers two in-between states, named in the menu header. **Starting** is a session on its way up. **Stuck** is Waydroid holding a leftover session after a stop that failed partway, which makes every new start fail with "Already tracking a session". In both, Start is disabled and Stop session (or a middle click) runs `waydroid session stop`, which clears a stuck session without root.

Click it (left or right) for the menu: Start session, Stop session, Stop container service, Freeze / Unfreeze, Show full UI, and an **Apps** submenu built from the launchers Waydroid already generates in `~/.local/share/applications`. Hidden apps (`NoDisplay=true`) stay hidden, and the list picks up installs and removals on the next poll. Middle click starts or stops the session.

**Stop container service** is enabled once the session is stopped and `waydroid-container.service` is still up. Stopping a session doesn't stop that service, and starting one D-Bus-activates it even when it's disabled at boot, so it sits there as root holding its memory until something stops it. The tray asks systemd to stop it over D-Bus, which polkit checks against `org.freedesktop.systemd1.manage-units`, so your desktop's authentication agent prompts for an admin password and remembers it for a few minutes. The tray never runs as root itself, and a dismissed prompt just gets you a notification.

Two toggles at the bottom of the menu, saved to `~/.config/waydroid-tray/config`:

- **Start session at login** runs `waydroid session start` when the tray starts, if no session is running. Starting the session brings the container service up on demand, so it can stay disabled at boot.
- **Hide icon while stopped** marks the icon passive while the session is stopped, so Plasma moves it to the hidden icons until a session starts. Panels without a hidden area, such as GNOME's, don't show it at all until then.

If a Waydroid command fails, you get a desktop notification with the end of its error output.

## Install

On x86_64 or arm64, this downloads the latest release binary. No Rust toolchain needed:

```sh
curl -fsSL https://raw.githubusercontent.com/karanshukla/waydroid-tray/main/install.sh | sh
```

It installs to `~/.local/bin` and adds the icons, a menu entry, and a systemd user unit (`waydroid-tray.service`) that starts the tray with your desktop session and restarts it if it crashes. The unit starts with `graphical-session.target` where your desktop has one, and from an autostart entry where it doesn't. To remove it:

```sh
curl -fsSL https://raw.githubusercontent.com/karanshukla/waydroid-tray/main/install.sh | sh -s -- --uninstall
```

To build from source instead, run `./install.sh` from a checkout (needs a Rust toolchain). `./install.sh --uninstall` works there too.

It's also [on crates.io](https://crates.io/crates/waydroid-tray), but `cargo install waydroid-tray` only gets you the binary, in `~/.cargo/bin`. The icons, systemd unit and menu entry come from `install.sh`, so without it the tray shows a generic icon and you start it yourself.

Re-running the installer restarts the tray on the new build, and `--uninstall` stops it. Stopping or restarting the unit leaves a running Waydroid session alone. Quit tray stops it until your next login.

## Desktop support

The tray is a [StatusNotifierItem](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/), so it shows up in any panel that hosts those:

- **KDE Plasma** has it built in.
- **GNOME** needs the [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/). Ubuntu ships it enabled.
- **XFCE, Cinnamon, Budgie, LXQt and COSMIC** have one, though on some it's a panel applet you add yourself.
- **Sway, Hyprland, niri** and other compositors without a panel of their own need a bar with a tray, such as [Waybar](https://github.com/Alexays/Waybar).

Compositors that don't run XDG autostart entries or reach `graphical-session.target` won't start the tray at login. Have them run this at startup instead, e.g. from an `exec` line in Sway's config:

```sh
systemctl --user import-environment WAYLAND_DISPLAY; systemctl --user start waydroid-tray
```

Two features need services your desktop may not run. Failure notifications need a notification daemon (e.g. mako or dunst), and **Stop container service** needs a polkit authentication agent (e.g. hyprpolkitagent or polkit-gnome) to ask for your password.

## Releasing

Bump `version` in `Cargo.toml`, then push a matching tag (`git tag v0.1.1 && git push origin v0.1.1`). The release workflow builds static x86_64 and arm64 binaries and attaches them to a GitHub release. The install script picks up the newest one. The same workflow then publishes that version to crates.io through [trusted publishing](https://crates.io/docs/trusted-publishing), so there's no token to keep around. It refuses to publish if the tag and `Cargo.toml` disagree.

## Why it doesn't use `waydroid status`

This is the one non-obvious bit. Waydroid registers `waydroid-container.service` for D-Bus activation, so _any_ call addressed to `id.waydro.Container` starts the service. `waydroid status` does exactly that. Polling it every 5 seconds would quietly restart the container service you might have disabled at boot on purpose.

**The tray checks whether the name has an owner first, and only then calls `GetSession` on the owner's unique bus name.** Checking status never starts anything. Start session and app launches still go through normal activation, which is what you want there.

## Why Rust

The first version was Python + PySide6. It worked, but Qt's tray implementation reports `ItemIsMenu=false` and drops the click position, so a left click could never open the menu. [ksni](https://github.com/iovxw/ksni) talks StatusNotifierItem directly and supports opening the menu on left click, and the binary doesn't drag Qt in at runtime.

## Running the tests

```sh
cargo install busd    # a D-Bus broker that needs no system config
cargo test
```

The integration tests in `tests/harness/` run the real tray against private system and session buses, with mock Waydroid, notification and tray host services and a `waydroid` shim on `PATH`. No Waydroid, Plasma or root needed.

`cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` both run in CI.

## Limits

- Session start and stop come from D-Bus name changes. A stop shows up straight away. A start takes a moment longer, because Waydroid claims its session name before the container has a session to report, so the tray re-checks every 0.5s until it does. Freezing isn't signalled, so while a session exists the tray polls every 5s to tell Running from Frozen. It doesn't poll Waydroid at all while stopped.
- If no session is running, `app launch` and `show-full-ui` start one and run it in the foreground. Waydroid exits 0 when that start fails, so those failures don't get a notification. `session start` does, because a real start never exits within 5s.
- Tested on Plasma 6 (Wayland) with Waydroid 1.6.3. The other desktops above should work, but I haven't tried them.
- It needs systemd: it runs as a user unit, and **Stop container service** goes through systemd's D-Bus API.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
