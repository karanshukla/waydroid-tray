#!/bin/sh
# Install waydroid-tray for the current user (binary, icons, systemd user unit,
# app menu and autostart entries). Run with --uninstall to remove it again.
#
# From a checkout it builds from source. Anywhere else, e.g. piped from curl,
# it downloads the latest release:
#   curl -fsSL https://raw.githubusercontent.com/karanshukla/waydroid-tray/main/install.sh | sh
set -eu

repo="karanshukla/waydroid-tray"
here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin/waydroid-tray"
icons="$HOME/.local/share/icons/hicolor/scalable/status"
app_icon="$HOME/.local/share/icons/hicolor/scalable/apps/waydroid-tray.svg"
unit="$HOME/.config/systemd/user/waydroid-tray.service"
launcher="$HOME/.local/share/applications/waydroid-tray.desktop"
# The unit starts with graphical-session.target, which many desktops never
# reach. Their autostart runs the launcher, which starts the unit instead.
autostart="$HOME/.config/autostart/waydroid-tray.desktop"

# Stops the unit and any tray started outside it, e.g. by an older version.
stop_tray() {
    systemctl --user stop waydroid-tray 2> /dev/null || true
    pkill -x -u "$(id -u)" waydroid-tray || return 0
    # Wait for it to exit and drop its lock.
    while pgrep -x -u "$(id -u)" waydroid-tray > /dev/null; do sleep 0.1; done
}

if [ "${1:-}" = "--uninstall" ]; then
    systemctl --user disable waydroid-tray 2> /dev/null || true
    stop_tray
    rm -f "$bin" "$unit" "$autostart" "$launcher" "$app_icon" "$icons"/waydroid-tray-*.svg
    systemctl --user daemon-reload 2> /dev/null || true
    echo "Removed waydroid-tray."
    exit 0
fi

# Piped into sh, $0 isn't this file, so there's nothing next to it to install.
if [ ! -f "$here/waydroid-tray.desktop" ]; then
    case "$(uname -m)" in
        x86_64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) echo "No prebuilt binary for $(uname -m). Build from a checkout instead." >&2; exit 1 ;;
    esac
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    echo "Downloading the latest waydroid-tray release for $arch..."
    curl -fsSL "https://github.com/$repo/releases/latest/download/waydroid-tray-$arch-linux.tar.gz" | tar -xzf - -C "$tmp"
    here="$tmp/waydroid-tray"
fi

# Release archives ship the binary. A checkout has Cargo.toml instead.
if [ -f "$here/Cargo.toml" ]; then
    cargo build --release --manifest-path "$here/Cargo.toml"
    built="$here/target/release/waydroid-tray"
else
    built="$here/waydroid-tray"
fi

# install(1) replaces the file in place, including an old symlink.
install -Dm755 "$built" "$bin"
# Drops icons an older version named differently.
rm -f "$icons"/waydroid-tray-*.svg
install -Dm644 -t "$icons" "$here"/icons/waydroid-tray-*.svg
install -Dm644 "$here/icons/waydroid-tray.svg" "$app_icon"
install -Dm644 "$here/waydroid-tray.service" "$unit"
install -Dm644 "$here/waydroid-tray.desktop" "$launcher"
install -Dm644 "$here/waydroid-tray.desktop" "$autostart"

systemctl --user daemon-reload
systemctl --user enable waydroid-tray
stop_tray
# Outside a desktop session (e.g. over ssh) there's no tray to show it in.
if systemctl --user is-active --quiet graphical-session.target || [ -n "${WAYLAND_DISPLAY:-}" ]; then
    # Waydroid needs it, and not every desktop passes it on to systemd.
    [ -n "${WAYLAND_DISPLAY:-}" ] && systemctl --user import-environment WAYLAND_DISPLAY
    systemctl --user start waydroid-tray
    echo "Installed and started the tray."
else
    echo "Installed. The tray starts with your next desktop session."
fi
