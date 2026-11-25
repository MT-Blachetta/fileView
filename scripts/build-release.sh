#!/usr/bin/env bash
# Builds the Linux release executable into ./dist. Needs only Rust
# (https://rustup.rs).
#
# The Windows release is built on Windows with the MSVC toolchain, see
# scripts/build-windows.ps1: it embeds the icon, version info and manifest,
# can sign the executable and packages the installer.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p dist

echo "==> Linux x86_64"
cargo build --release
cp target/release/planar dist/planar-linux-x86_64

ls -lh dist
