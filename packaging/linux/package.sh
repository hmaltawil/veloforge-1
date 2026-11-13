#!/usr/bin/env bash
# Package the built Linux program for a release:
#   dist/VeloForge-<version>-linux-x64.tar.gz  (any distribution, ./install.sh)
#   dist/veloforge_<version>-1_amd64.deb       (Ubuntu, Debian, Mint, Pop!_OS)
# Run after `cargo build --release`; needs cargo-deb. Used by
# .github/workflows/release.yml, which builds on Ubuntu 22.04.
set -euo pipefail
cd "$(dirname "$0")/../.."

version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
program="${CARGO_TARGET_DIR:-target}/release/veloforge"

# Refuse a program that needs a newer C library than Ubuntu 22.04's, so the
# downloads keep working on older distributions.
newest=$(objdump -T "$program" | grep -oE 'GLIBC_[0-9.]+' | sort -Vu | tail -1)
if [ "$(printf '%s\n' "$newest" GLIBC_2.35 | sort -V | tail -1)" != GLIBC_2.35 ]; then
    echo "error: $program needs $newest; build it on Ubuntu 22.04 (glibc 2.35)" >&2
    exit 1
fi
echo "Needs $newest at most: runs on Ubuntu 22.04, Debian 12, Fedora 36 and newer"

mkdir -p dist
rm -rf dist/VeloForge
mkdir -p dist/VeloForge/packaging/icons dist/VeloForge/packaging/linux
strip -o dist/VeloForge/veloforge "$program"
cp install.sh LICENSE packaging/linux/README-Linux.txt dist/VeloForge/
cp packaging/veloforge.desktop dist/VeloForge/packaging/
cp packaging/linux/veloforge-update dist/VeloForge/packaging/linux/
cp packaging/icons/veloforge-{48,64,128,256}.png packaging/icons/veloforge.svg \
    dist/VeloForge/packaging/icons/
tar -C dist --owner=0 --group=0 -czf "dist/VeloForge-$version-linux-x64.tar.gz" VeloForge
rm -rf dist/VeloForge

cargo deb --no-build --output "dist/veloforge_${version}-1_amd64.deb"
ls -l dist
