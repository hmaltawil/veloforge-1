#!/usr/bin/env bash
# Install a Linux download on this fresh system and check that VeloForge
# starts. Run as root in a throwaway container (the release workflow does):
#   packaging/linux/smoke-test.sh deb|tar DIST_FOLDER
set -euo pipefail
kind=$1
dist=$(cd "$2" && pwd)

# A virtual screen to start the app on, and a software OpenGL driver.
if command -v apt-get >/dev/null; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq xvfb libgl1-mesa-dri >/dev/null
elif command -v dnf >/dev/null; then
    # The window libraries every desktop has (the .tar.gz cannot pull them in).
    dnf install -y -q tar gzip xorg-x11-server-Xvfb mesa-dri-drivers dbus-libs \
        libX11-xcb libXcursor libXrandr libXi libxkbcommon-x11 >/dev/null
fi

case $kind in
    deb)
        apt-get install -y -qq "$dist"/veloforge_*_amd64.deb >/dev/null
        program=/usr/bin/veloforge
        # Each recommended package (one of each "a | b" choice) exists here
        # and was installed.
        dpkg-deb -f "$dist"/veloforge_*_amd64.deb Recommends | tr ',' '\n' | while read -r choice; do
            ok=no
            for p in $(echo "$choice" | tr '|' ' '); do
                if dpkg -s "$p" >/dev/null 2>&1; then ok=yes; fi
            done
            if [ $ok = no ]; then
                echo "recommended package not installed: $choice" >&2
                exit 1
            fi
        done
        test -f /usr/share/applications/veloforge.desktop
        test -f /usr/share/icons/hicolor/256x256/apps/veloforge.png
        ;;
    tar)
        work=$(mktemp -d)
        tar -xzf "$dist"/VeloForge-*-linux-x64.tar.gz -C "$work"
        "$work/VeloForge/install.sh"
        program="$HOME/.local/bin/veloforge"
        grep -qx "Exec=$program" "$HOME/.local/share/applications/veloforge.desktop"
        test -f "$HOME/.local/share/icons/hicolor/256x256/apps/veloforge.png"
        ;;
    *)
        echo "usage: $0 deb|tar DIST_FOLDER" >&2
        exit 2
        ;;
esac

# Start it in demo mode: it should still be running 20 seconds later.
# Xvfb picks a free display number and writes it out once it is ready.
number=$(mktemp)
Xvfb -displayfd 3 -screen 0 1360x860x24 3>"$number" >/dev/null 2>&1 &
for _ in $(seq 100); do
    if [ -s "$number" ]; then break; fi
    sleep 0.1
done
data=$(mktemp -d)
set +e
DISPLAY=":$(cat "$number")" VELOFORGE_DATA="$data" timeout 20 "$program" --demo
code=$?
set -e
cat "$data/veloforge.log"
if [ "$code" -ne 124 ]; then
    echo "VeloForge stopped early (exit code $code)" >&2
    exit 1
fi
grep -q "VeloForge .* started" "$data/veloforge.log"

if [ "$kind" = tar ]; then
    "$work/VeloForge/install.sh" --uninstall
    test ! -e "$program"
    test ! -e "$HOME/.local/share/applications/veloforge.desktop"
fi
echo "OK: the $kind package installs and VeloForge starts"
