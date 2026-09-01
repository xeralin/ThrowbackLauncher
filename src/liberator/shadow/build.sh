#!/bin/sh
set -e
cd "$(dirname "$0")"
RUSTFLAGS="-C link-arg=-nostdlib -C link-arg=-Wl,--entry,DllMain" \
    cargo build --release --target x86_64-pc-windows-gnu
cp target/x86_64-pc-windows-gnu/release/shadow.dll Shadow.dll
echo "shadow -> Shadow.dll"
