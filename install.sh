#!/bin/sh
# Install waydroid-tray for the current user (binary, icons, systemd user unit,
# app menu and autostart entries). Run with --uninstall to remove it again.
#
# From a checkout it builds from source. Anywhere else, e.g. piped from curl,
# it downloads the latest release:
#   curl -fsSL https://raw.githubusercontent.com/karanshukla/waydroid-tray/main/install.sh | sh
set -eu

repo="karanshukla/waydroid-tray"
bin="$HOME/.local/bin/waydroid-tray"

# Piped into sh, $0 is the shell rather than this file, and dirname "$0" is
# whatever directory curl ran in. Only trust the files next to this script
# when it really is one: a checkout, or an unpacked release archive.
here=
if [ -f "$0" ] && [ "$(basename "$0")" = install.sh ]; then
    dir=$(cd "$(dirname "$0")" && pwd)
    if grep -qx 'name = "waydroid-tray"' "$dir/Cargo.toml" 2>/dev/null ||
        { [ -f "$dir/waydroid-tray" ] && [ -f "$dir/waydroid-tray.desktop" ]; }; then
        here=$dir
    fi
fi

if [ -z "$here" ]; then
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

# The installed binary may predate --uninstall.
if [ "${1:-}" = "--uninstall" ]; then
    "$built" --uninstall
    rm -f "$bin"
    exit 0
fi

# install(1) replaces the file in place, including an old symlink.
install -Dm755 "$built" "$bin"
"$bin" --install
