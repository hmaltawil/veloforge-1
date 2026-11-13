#!/usr/bin/env bash
# Install VeloForge for the current user, with an app-menu entry and icon.
#
#   ./install.sh              install, or update to this version
#   ./install.sh --uninstall  remove the program, menu entry and icons
#                             (your settings and rides are kept)
#
# It also installs `veloforge-update`, which updates to the newest release
# with one command.
#
# In the downloaded VeloForge-*-linux-x64.tar.gz this installs the ready-built
# program. In a copy of the source code it builds it first, which needs Rust;
# run it again after `git pull` to update.
set -euo pipefail
cd "$(dirname "$0")"

bin="$HOME/.local/bin/veloforge"
icons="$HOME/.local/share/icons/hicolor"
apps="$HOME/.local/share/applications"

case "${1:-}" in
    "") ;;
    --uninstall)
        rm -f "$bin" "$bin-update" "$apps/veloforge.desktop" "$icons/scalable/apps/veloforge.svg"
        for size in 48 64 128 256; do
            rm -f "$icons/${size}x${size}/apps/veloforge.png"
        done
        gtk-update-icon-cache -q "$icons" 2>/dev/null || true
        update-desktop-database "$apps" 2>/dev/null || true
        echo "VeloForge removed. Your settings and rides are still in ~/.local/share/veloforge/."
        exit 0
        ;;
    *)
        echo "Usage: ./install.sh [--uninstall]" >&2
        exit 2
        ;;
esac

if [ -f Cargo.toml ]; then
    cargo build --release
    program="${CARGO_TARGET_DIR:-target}/release/veloforge"
else
    program=./veloforge
fi
install -Dm755 "$program" "$bin"
install -Dm755 packaging/linux/veloforge-update "$bin-update"

# Desktop menus do not always search ~/.local/bin, so use the full path.
for size in 48 64 128 256; do
    install -Dm644 "packaging/icons/veloforge-$size.png" "$icons/${size}x${size}/apps/veloforge.png"
done
install -Dm644 packaging/icons/veloforge.svg "$icons/scalable/apps/veloforge.svg"
gtk-update-icon-cache -q "$icons" 2>/dev/null || true

mkdir -p "$apps"
sed "s|^Exec=.*|Exec=$bin|" packaging/veloforge.desktop > "$apps/veloforge.desktop"
update-desktop-database "$apps" 2>/dev/null || true

echo
echo "VeloForge installed. Open it from your app menu, or run: $bin"
echo "To update to a new release later, run: veloforge-update"
