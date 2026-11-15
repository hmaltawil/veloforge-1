#!/usr/bin/env bash
# Open the .dmg, copy VeloForge.app out of it the way a person would, check
# the bundle and start it: it should still be running 20 seconds later.
#   packaging/macos/smoke-test.sh DIST_FOLDER
set -euo pipefail
dist=$(cd "$1" && pwd)
dmg=$(ls "$dist"/VeloForge-*-macos.dmg)
mnt=$(mktemp -d)
hdiutil attach -nobrowse -readonly -mountpoint "$mnt" "$dmg" >/dev/null
apps=$(mktemp -d)
cp -R "$mnt/VeloForge.app" "$apps/"
test -L "$mnt/Applications"
test -f "$mnt/README-macOS.txt"
hdiutil detach "$mnt" >/dev/null

app="$apps/VeloForge.app"
codesign --verify --deep --strict "$app"
archs=$(lipo -archs "$app/Contents/MacOS/veloforge")
echo "architectures: $archs"
case "$archs" in *x86_64*arm64* | *arm64*x86_64*) ;; *)
    echo "VeloForge.app is not universal: $archs" >&2
    exit 1
    ;;
esac
version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
test "$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")" = "$version"
/usr/libexec/PlistBuddy -c 'Print NSBluetoothAlwaysUsageDescription' "$app/Contents/Info.plist" >/dev/null
test -f "$app/Contents/Resources/veloforge.icns"

data=$(mktemp -d)
VELOFORGE_DATA="$data" "$app/Contents/MacOS/veloforge" --demo &
pid=$!
sleep 20
if ! kill -0 "$pid" 2>/dev/null; then
    set +e
    wait "$pid"
    code=$?
    set -e
    cat "$data/veloforge.log" || true
    echo "VeloForge stopped early (exit code $code)" >&2
    exit 1
fi
kill "$pid"
wait "$pid" 2>/dev/null || true
cat "$data/veloforge.log"
grep -q "VeloForge .* started" "$data/veloforge.log"
echo "OK: the .dmg opens and VeloForge starts"
