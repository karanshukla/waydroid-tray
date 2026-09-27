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

# The installed binary may predate --uninstall.
if [ "${1:-}" = "--uninstall" ]; then
    "$built" --uninstall
    rm -f "$bin"
    exit 0
fi

# install(1) replaces the file in place, including an old symlink.
install -Dm755 "$built" "$bin"
"$bin" --install
