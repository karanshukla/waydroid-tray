#!/bin/sh
# Install waydroid-tray for the current user (binary, icons, autostart + app
# menu entry). Run with --uninstall to remove it again.
#
# From a checkout it builds from source. Anywhere else, e.g. piped from curl,
# it downloads the latest release:
#   curl -fsSL https://raw.githubusercontent.com/karanshukla/waydroid-tray/main/install.sh | sh
set -eu

repo="karanshukla/waydroid-tray"
here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin/waydroid-tray"
icons="$HOME/.local/share/icons/hicolor/scalable/status"
autostart="$HOME/.config/autostart/waydroid-tray.desktop"
launcher="$HOME/.local/share/applications/waydroid-tray.desktop"

# Only this user's trays; exits non-zero if none was running.
stop_tray() {
    pkill -x -u "$(id -u)" waydroid-tray || return 1
    # Wait for it to exit and drop its lock.
    while pgrep -x -u "$(id -u)" waydroid-tray > /dev/null; do sleep 0.1; done
}

if [ "${1:-}" = "--uninstall" ]; then
    stop_tray || true
    rm -f "$bin" "$autostart" "$launcher" "$icons"/waydroid-tray-*.svg
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
install -Dm644 -t "$icons" "$here"/icons/waydroid-tray-*.svg
mkdir -p "$(dirname "$autostart")" "$(dirname "$launcher")"
for target in "$autostart" "$launcher"; do
    sed "s|@BIN@|$bin|" "$here/waydroid-tray.desktop" > "$target"
done
# Restart a running tray so the new binary takes effect.
if stop_tray; then
    nohup "$bin" > /dev/null 2>&1 &
    echo "Installed and restarted the running tray."
else
    echo "Installed. Start it now with: waydroid-tray &"
fi
