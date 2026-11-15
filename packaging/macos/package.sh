#!/usr/bin/env bash
# Build VeloForge.app for Apple Silicon and Intel Macs in one (universal)
# program, and put it in a disk image: dist/VeloForge-<version>-macos.dmg.
# Runs on a Mac with Rust, both targets and the Xcode command-line tools
# (the release workflow does this on GitHub's macOS machines).
set -euo pipefail
cd "$(dirname "$0")/../.."
version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
export MACOSX_DEPLOYMENT_TARGET=11.0

for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked --target "$target"
done

rm -rf dist/macos
stage=dist/macos/VeloForge
app="$stage/VeloForge.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/veloforge" \
    target/aarch64-apple-darwin/release/veloforge \
    target/x86_64-apple-darwin/release/veloforge
strip -x "$app/Contents/MacOS/veloforge"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
cp packaging/macos/veloforge.icns "$app/Contents/Resources/"
printf 'APPL????' > "$app/Contents/PkgInfo"
# Not signed with an Apple developer certificate, but signed "ad hoc" so
# that Apple Silicon Macs run it and Bluetooth permission sticks.
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"
lipo -archs "$app/Contents/MacOS/veloforge"

# Drag the app onto Applications, as usual.
ln -s /Applications "$stage/Applications"
cp packaging/macos/README-macOS.txt "$stage/"
cp LICENSE "$stage/LICENSE.txt"
mkdir -p dist
hdiutil create -volname "VeloForge $version" -srcfolder "$stage" -ov -format UDZO \
    "dist/VeloForge-$version-macos.dmg"
ls -l dist/*.dmg
