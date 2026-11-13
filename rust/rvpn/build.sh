#!/bin/sh
set -e
cd "$(dirname "$0")"
linux=x86_64-unknown-linux-gnu
win32=i686-pc-windows-msvc
win64=x86_64-pc-windows-msvc
cargo build --release --locked --target $linux -p tap_bridge -p net_shim
cargo build --release --locked --target $win32 -p adapter_hook -p rvpn_launcher -p netsh -p drvinst
cargo build --release --locked --target $win64 -p netsh -p rvpnnetmp
out=../../rvpn
mkdir -p "$out"
cp target/$linux/release/tap_bridge "$out/"
cp target/$linux/release/libnet_shim.so "$out/net_shim.so"
cp target/$win32/release/adapter_hook.dll "$out/"
cp target/$win32/release/rvpn_launcher.exe "$out/"
cp target/$win32/release/netsh.exe "$out/"
cp target/$win32/release/drvinst.exe "$out/"
cp target/$win64/release/netsh.exe "$out/netsh64.exe"
cp target/$win64/release/rvpnnetmp.dll "$out/rvpnnetmp.sys"
cp assets/* "$out/"
echo "built + deployed -> $out"
