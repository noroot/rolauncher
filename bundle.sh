#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds target/release/RoLauncher.app (ad-hoc signed).
set -euo pipefail
cd "$(dirname "$0")"

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/release/RoLauncher.app

cargo build --release
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/rolauncher "$APP/Contents/MacOS/"
cp resources/AppIcon.icns "$APP/Contents/Resources/"
sed "s/__VERSION__/$VERSION/g" resources/Info.plist > "$APP/Contents/Info.plist"
codesign --force --sign - "$APP" >/dev/null
echo "Built $APP"
