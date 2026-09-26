#!/bin/sh
set -e
cd "$(dirname "$0")"
make
mkdir -p ../../rvpn
cp build/* ../../rvpn/
cp assets/OpenSans-Regular.ttf assets/OpenSans-Bold.ttf assets/LICENSE \
   ../../rvpn/
echo "built + deployed -> ../../rvpn/"
