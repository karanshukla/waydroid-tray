# waydroid-tray

A small system tray icon for [Waydroid](https://waydro.id) on KDE Plasma. Waydroid doesn't ship one, so once it's running in the background there's no way to see its state or stop it without a terminal.

The icon is a monochrome Android head that follows your Plasma colour scheme, like the Breeze icons next to it:

| Icon | State |
|---|---|
| Outline | Session stopped |
| Filled, eyes closed | Frozen (no Android windows open, 0% CPU but still holding RAM) |
| Filled, eyes open | Running |

Click it (left or right) for the menu: Start session, Stop session, Show full UI, and an **Apps** submenu built from the launchers Waydroid already generates in `~/.local/share/applications`. Hidden apps (`NoDisplay=true`) stay hidden, and the list picks up installs and removals on the next poll.

## Install

Needs a Rust toolchain.

```sh
./install.sh          # builds, installs to ~/.local/bin, adds icons + autostart + menu entry
waydroid-tray &       # start it now without logging out
./install.sh --uninstall
```

## Why it doesn't use `waydroid status`

This is the one non-obvious bit. Waydroid registers `waydroid-container.service` for D-Bus activation, so _any_ call addressed to `id.waydro.Container` starts the service. `waydroid status` does exactly that. Polling it every 5 seconds would quietly restart the container service you might have disabled at boot on purpose.

**The tray checks whether the name has an owner first, and only then calls `GetSession` on the owner's unique bus name.** Checking status never starts anything. Start session and app launches still go through normal activation, which is what you want there.

## Why Rust

The first version was Python + PySide6. It worked, but Qt's tray implementation reports `ItemIsMenu=false` and drops the click position, so a left click could never open the menu. [ksni](https://github.com/iovxw/ksni) talks StatusNotifierItem directly and supports opening the menu on left click, and the binary doesn't drag Qt in at runtime.

## Limits

- Polls every 5s. Waydroid doesn't emit a signal when the session state changes, so there's nothing to subscribe to.
- Tested on Plasma 6 (Wayland) with Waydroid 1.6.3. Other StatusNotifierItem hosts should work but I haven't tried them.
