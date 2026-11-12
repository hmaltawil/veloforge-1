#!/usr/bin/env bash
# Build VeloForge and install it for the current user, with an app-menu entry.
# Run it again after `git pull` to update.
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release
install -Dm755 target/release/veloforge "$HOME/.local/bin/veloforge"

# Desktop menus do not always search ~/.local/bin, so use the full path.
icons="$HOME/.local/share/icons/hicolor"
for size in 48 64 128 256; do
    install -Dm644 "packaging/icons/veloforge-$size.png" "$icons/${size}x${size}/apps/veloforge.png"
done
install -Dm644 packaging/icons/veloforge.svg "$icons/scalable/apps/veloforge.svg"
gtk-update-icon-cache -q "$icons" 2>/dev/null || true

apps="$HOME/.local/share/applications"
mkdir -p "$apps"
sed "s|^Exec=.*|Exec=$HOME/.local/bin/veloforge|" packaging/veloforge.desktop > "$apps/veloforge.desktop"
update-desktop-database "$apps" 2>/dev/null || true

echo
echo "VeloForge installed. Open it from your app menu, or run: $HOME/.local/bin/veloforge"
