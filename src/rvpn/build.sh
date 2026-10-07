#!/bin/sh
set -e
cd "$(dirname "$0")"
make
mkdir -p ../../rvpn
cp build/* assets/* ../../rvpn/
echo "built + deployed -> ../../rvpn/"
