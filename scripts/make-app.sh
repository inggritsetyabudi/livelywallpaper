#!/bin/bash
#
# Assemble Lively.app from the release binary.
#
# A bare Mach-O executable cannot show a Dock icon, own a menu bar item, or be
# launched from Finder — macOS requires the bundle layout. This script produces
# one next to the project.
#
#   ./scripts/make-app.sh          build the bundle
#   ./scripts/make-app.sh --run    build, launch it, and stop any previous copy

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APP_NAME="Lively"
BUNDLE_ID="com.lively.wallpaper"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
VERSION="${VERSION:-0.1.0}"

DIST="$ROOT/dist"
APP="$DIST/$APP_NAME.app"

echo "==> Building release binary"
cargo build --release

# The icon is generated, not committed as binary art, so it can be re-derived
# when the palette changes. Regenerate only when missing to keep rebuilds fast.
if [ ! -f assets/AppIcon.icns ] || [ scripts/make-icon.swift -nt assets/AppIcon.icns ]; then
  echo "==> Generating app icon"
  swift scripts/make-icon.swift
fi

echo "==> Assembling $APP_NAME.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

install -m 0755 "target/release/livelywallpaper" "$APP/Contents/MacOS/$APP_NAME"

# An .icns is what Finder and the Dock actually read.
if [ -f assets/AppIcon.icns ]; then
  cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
fi

# Built-in wallpapers are embedded in the binary at compile time, but they are
# also shipped as files so a future "reveal in Finder" can point at them.
if [ -d resources/wallpapers ]; then
  mkdir -p "$APP/Contents/Resources/Wallpapers"
  cp -R resources/wallpapers/. "$APP/Contents/Resources/Wallpapers/"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>Lively Wallpaper</string>
    <key>CFBundleExecutable</key>
    <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>LSMinimumSystemVersion</key>
    <string>14.0</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <!-- Menu-bar app: no Dock icon at launch. The window switches the
         activation policy to regular on demand. -->
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
PLIST

# Ad-hoc signature: unsigned bundles are killed on launch on Apple silicon.
echo "==> Signing (ad-hoc)"
codesign --force --deep --sign - "$APP" 2>/dev/null || echo "   (codesign unavailable; the app will still run locally)"

echo "==> Built $APP"

if [ "${1:-}" = "--run" ]; then
  echo "==> Launching"
  # Stop any previous copy first. Launching a second daemon would fail to bind
  # the socket *after* it had already created desktop windows, leaving a dead
  # process and a confusing "no daemon" error for the commands that follow.
  pkill -f "$APP/Contents/MacOS/$APP_NAME" 2>/dev/null || true
  sleep 1

  # Launched through `open` so macOS treats it as a real app: the binary sees a
  # bundle path, starts the daemon and opens the settings window by itself.
  open "$APP"
  sleep 4

  if "$APP/Contents/MacOS/$APP_NAME" list-library >/dev/null 2>&1; then
    echo "==> Running. Library:"
    "$APP/Contents/MacOS/$APP_NAME" list-library | sed 's/^/    /'
  else
    echo "==> WARNING: the daemon is not answering after launch" >&2
    exit 1
  fi
  echo "==> Logs: log stream --predicate 'process == \"Lively\"'"
fi
