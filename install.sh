#!/bin/sh
# Install waydroid-tray for the current user (binary, icons, systemd user unit,
# app menu and autostart entries). Run with --uninstall to remove it again.
#
# From a checkout it builds from source. Anywhere else, e.g. piped from curl,
# it downloads the latest release, or the one WAYDROID_TRAY_VERSION names
# (e.g. WAYDROID_TRAY_VERSION=v1.2.3):
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
    version=${WAYDROID_TRAY_VERSION:-latest}
    case "$version" in
        *[!A-Za-z0-9.+-]*) echo "WAYDROID_TRAY_VERSION=$version isn't a release tag like v0.3.0." >&2; exit 1 ;;
        [0-9]*) version="v$version" ;;
    esac
    if [ "$version" = latest ]; then
        url="https://github.com/$repo/releases/latest/download"
    else
        url="https://github.com/$repo/releases/download/$version"
    fi
    tarball="waydroid-tray-$arch-linux.tar.gz"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    echo "Downloading waydroid-tray $version for $arch..."
    curl -fsSL -o "$tmp/$tarball" "$url/$tarball"
    # Releases before SHA256SUMS was published can't be checked, so they
    # aren't installed this way at all.
    if ! curl -fsSL -o "$tmp/SHA256SUMS" "$url/SHA256SUMS"; then
        echo "Couldn't download SHA256SUMS for waydroid-tray $version, so the download can't be verified." >&2
        echo "Pick a newer release with WAYDROID_TRAY_VERSION, or build from a checkout with ./install.sh." >&2
        exit 1
    fi
    count=$(awk -v f="$tarball" '$2 == f { n++ } END { print n + 0 }' "$tmp/SHA256SUMS")
    if [ "$count" -ne 1 ]; then
        echo "SHA256SUMS lists $tarball $count times, expected once." >&2
        exit 1
    fi
    if ! (cd "$tmp" && awk -v f="$tarball" '$2 == f' SHA256SUMS | sha256sum -c --strict --quiet -); then
        echo "$tarball doesn't match its SHA256SUMS entry. Not installing it." >&2
        exit 1
    fi
    tar -xzf "$tmp/$tarball" -C "$tmp"
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
