#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
mkdir -p target/appimage dist
curl --fail --location --retry 3 \
  https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage \
  --output target/appimage/linuxdeploy.AppImage
chmod +x target/appimage/linuxdeploy.AppImage
install -Dm644 assets/icon.svg target/appimage/tap.svg
export ARCH=x86_64
export APPIMAGE_EXTRACT_AND_RUN=1
export OUTPUT="$PWD/dist/Tap-${version}-linux-x86_64.AppImage"
target/appimage/linuxdeploy.AppImage --appimage-extract-and-run \
  --appdir target/appimage/Tap.AppDir \
  --executable target/release/tap-autoclicker \
  --desktop-file packaging/tap.desktop \
  --icon-file target/appimage/tap.svg \
  --output appimage
chmod +x "$OUTPUT"
