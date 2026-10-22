#!/usr/bin/env sh

set -ex

# cross compile windows version
cargo build --target x86_64-pc-windows-gnu --release

# cleanup cross compiled windows artifacts
(cd deps && ./clean.sh)

# build linux versions
cargo deb -- --features=va-static

# check if installing works
dpkg -i target/debian/Proximity*.deb
cp target/release/proximity target/release/proximity_va_static

# build version with dynamic libva
cargo build --release

mkdir packages

PKGDIR="$PWD/packages"

# package windows
(
  cd target/x86_64-pc-windows-gnu/release/
  zip proximity-windows.zip proximity.exe
  mv proximity-windows.zip "$PKGDIR/"
)

# package linux
(
  cp target/debian/Proximity*.deb "$PKGDIR/"
  cp proximity.desktop target/release/
  cd target/release/
  zip proximity-linux.zip proximity proximity_va_static proximity.desktop
  mv proximity-linux.zip "$PKGDIR/"
)
