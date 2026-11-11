#!/bin/zsh
# Regenerates the iOS app icon sizes from src-tauri/icons/memoir-ios.svg into
# the Xcode project, without an alpha channel (App Store Connect rejects
# icons that have one, even when every pixel is opaque).
#
# Run from anywhere:  app/scripts/ios-icons.sh
set -euo pipefail

app="${0:A:h:h}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cd "$app"
npx tauri icon src-tauri/icons/memoir-ios.svg -o "$tmp" --ios-color "#1F3A5F" >/dev/null
swift scripts/flatten-png.swift "$tmp"/ios/*.png
cp "$tmp"/ios/*.png src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset/
echo "Updated src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset"
