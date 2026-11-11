#!/bin/zsh
# Builds Memoir for iPhone as an App Store build and uploads it to App Store
# Connect, where it shows up in TestFlight. See docs/RELEASE.md (iOS).
#
#   app/scripts/ios-testflight.sh           # build, then upload
#   app/scripts/ios-testflight.sh --no-upload
#                                           # build and export the .ipa only
#
# Signing and the upload use the Apple account signed in to Xcode
# (Settings → Accounts), with the team in src-tauri/tauri.ios.conf.json.
# Every upload needs a new build number: it's the time of the build
# (YYYYMMDDHHMM), so it always goes up.
set -euo pipefail

upload=1
[[ "${1:-}" == "--no-upload" ]] && upload=0

# Rust and Node.js, installed for this user.
export PATH="$HOME/.cargo/bin:$HOME/.local/node/bin:$PATH"

app="${0:A:h:h}"
apple="$app/src-tauri/gen/apple"
team="$(sed -n 's/.*"developmentTeam": *"\([^"]*\)".*/\1/p' "$app/src-tauri/tauri.ios.conf.json")"
build_number="$(date +%Y%m%d%H%M)"

archive="$apple/build/memoir-app_iOS.xcarchive"
out="$apple/build/testflight"
plist="$apple/memoir-app_iOS/Info.plist"
tmp="$(mktemp -d)"
export_options="$tmp/ExportOptions.plist"
# The build writes the build number into the checked-in Info.plist; put it back.
cp "$plist" "$tmp/Info.plist"
trap 'cp "$tmp/Info.plist" "$plist"; rm -rf "$tmp"' EXIT

cd "$app"
rm -rf "$archive"
npm run tauri ios build -- --archive-only \
  --config "{\"bundle\":{\"iOS\":{\"bundleVersion\":\"$build_number\"}}}"

cat > "$export_options" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>$( ((upload)) && echo upload || echo export )</string>
  <key>teamID</key><string>$team</string>
  <key>signingStyle</key><string>automatic</string>
  <key>manageAppVersionAndBuildNumber</key><false/>
</dict>
</plist>
PLIST

rm -rf "$out"
xcodebuild -exportArchive -archivePath "$archive" -exportPath "$out" \
  -exportOptionsPlist "$export_options" -allowProvisioningUpdates

if ((upload)); then
  echo "Uploaded build $build_number. It shows up in App Store Connect → TestFlight"
  echo "after Apple finishes processing it (usually 5-15 minutes)."
else
  echo "Built $out/Memoir.ipa (build $build_number), not uploaded."
fi
