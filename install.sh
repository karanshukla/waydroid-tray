#!/bin/sh
# Build and install waydroid-tray for the current user (binary, icons,
# autostart + app menu entry). Run with --uninstall to remove it again.
set -eu

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

cargo build --release --manifest-path "$here/Cargo.toml"

# install(1) replaces the file in place, including an old symlink.
install -Dm755 "$here/target/release/waydroid-tray" "$bin"
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
