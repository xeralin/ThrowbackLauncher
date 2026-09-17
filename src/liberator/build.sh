#!/bin/sh
set -e
cd "$(dirname "$0")"
sh shadow/build.sh
cargo build --release --target x86_64-pc-windows-gnu
cp target/x86_64-pc-windows-gnu/release/Liberator.exe ../../Liberator.exe
echo "liberator -> ../../Liberator.exe"
