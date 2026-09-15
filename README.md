# waydroid-tray

A small system tray icon for [Waydroid](https://waydro.id) on KDE Plasma. Waydroid doesn't ship one, so once it's running in the background there's no way to see its state or stop it without a terminal.

The icon is a monochrome Android head that follows your Plasma colour scheme, like the Breeze icons next to it:

| Icon | State |
|---|---|
| Outline | Session stopped |
| Filled, eyes closed | Frozen (no Android windows open, 0% CPU but still holding RAM) |
| Filled, eyes open | Running |

The outline also covers two in-between states, named in the menu header. **Starting** is a session on its way up. **Stuck** is Waydroid holding a leftover session after a stop that failed partway, which makes every new start fail with "Already tracking a session". In both, Start is disabled and Stop session (or a middle click) runs `waydroid session stop`, which clears a stuck session without root.

Click it (left or right) for the menu: Start session, Stop session, Freeze / Unfreeze, Show full UI, and an **Apps** submenu built from the launchers Waydroid already generates in `~/.local/share/applications`. Hidden apps (`NoDisplay=true`) stay hidden, and the list picks up installs and removals on the next poll. Middle click starts or stops the session.

Two toggles at the bottom of the menu, saved to `~/.config/waydroid-tray/config`:

- **Start session at login** runs `waydroid session start` when the tray starts, if no session is running. Starting the session brings the container service up on demand, so it can stay disabled at boot.
- **Hide icon while stopped** marks the icon passive while the session is stopped, so Plasma moves it to the hidden icons until a session starts.

If a Waydroid command fails, you get a desktop notification with the end of its error output.

## Install

Needs a Rust toolchain.

```sh
./install.sh          # builds, installs to ~/.local/bin, adds icons + autostart + menu entry
waydroid-tray &       # start it now without logging out
./install.sh --uninstall
```

Re-running `./install.sh` restarts a running tray on the new build, and `--uninstall` stops it.

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

The integration tests in `tests/harness.rs` run the real tray against private system and session buses, with mock Waydroid, notification and tray host services and a `waydroid` shim on `PATH`. No Waydroid, Plasma or root needed.

## Limits

- Session start and stop come from D-Bus name changes. A stop shows up straight away. A start takes a moment longer, because Waydroid claims its session name before the container has a session to report, so the tray re-checks every 0.5s until it does. Freezing isn't signalled, so while a session exists the tray polls every 5s to tell Running from Frozen. It doesn't poll Waydroid at all while stopped.
- If no session is running, `app launch` and `show-full-ui` start one and run it in the foreground. Waydroid exits 0 when that start fails, so those failures don't get a notification. `session start` does, because a real start never exits within 5s.
- Tested on Plasma 6 (Wayland) with Waydroid 1.6.3. Other StatusNotifierItem hosts should work but I haven't tried them.
