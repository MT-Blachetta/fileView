#!/usr/bin/env bash
# Builds release executables for Linux and Windows into ./dist.
#
# Linux:   needs only Rust (https://rustup.rs).
# Windows: cross-compiled with the x86_64-pc-windows-gnu target. Either
#          install mingw-w64 (`sudo apt install mingw-w64`) or zig +
#          cargo-zigbuild (`cargo install cargo-zigbuild`, zig on PATH).
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p dist

echo "==> Linux x86_64"
cargo build --release
cp target/release/planar dist/planar-linux-x86_64

echo "==> Windows x86_64"
rustup target add x86_64-pc-windows-gnu >/dev/null
if command -v x86_64-w64-mingw32-gcc >/dev/null; then
    cargo build --release --target x86_64-pc-windows-gnu
elif command -v cargo-zigbuild >/dev/null && command -v zig >/dev/null; then
    cargo zigbuild --release --target x86_64-pc-windows-gnu
else
    echo "No Windows linker found: install mingw-w64 or zig + cargo-zigbuild" >&2
    exit 1
fi
cp target/x86_64-pc-windows-gnu/release/planar.exe dist/planar-windows-x86_64.exe

ls -lh dist
