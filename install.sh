#!/bin/sh
# Build and install waydroid-tray for the current user (binary, icons,
# autostart + app menu entry). Run with --uninstall to remove it again.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin/waydroid-tray"
icons="$HOME/.local/share/icons/hicolor/scalable/status"
autostart="$HOME/.config/autostart/waydroid-tray.desktop"
launcher="$HOME/.local/share/applications/waydroid-tray.desktop"

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$bin" "$autostart" "$launcher" "$icons"/waydroid-tray-*.svg
    echo "Removed waydroid-tray. Quit the running tray from its menu."
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
echo "Installed. Start it now with: waydroid-tray &"
