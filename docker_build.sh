#!/usr/bin/env sh

set -ex

# cross compile windows version
cargo build --target x86_64-pc-windows-gnu --release

# cleanup cross compiled windows artifacts
(cd deps && ./clean.sh)

# build linux versions
cargo deb -- --features=va-static

# check if installing works
dpkg -i target/debian/KoshkaRemote*.deb
cp target/release/koshkaremote target/release/koshkaremote_va_static

# build version with dynamic libva
cargo build --release

mkdir packages

PKGDIR="$PWD/packages"

# package windows
(
  cd target/x86_64-pc-windows-gnu/release/
  zip koshkaremote-windows.zip koshkaremote.exe
  mv koshkaremote-windows.zip "$PKGDIR/"
)

# package linux
(
  cp target/debian/KoshkaRemote*.deb "$PKGDIR/"
  cp koshkaremote.desktop target/release/
  cd target/release/
  zip koshkaremote-linux.zip koshkaremote koshkaremote_va_static koshkaremote.desktop
  mv koshkaremote-linux.zip "$PKGDIR/"
)
