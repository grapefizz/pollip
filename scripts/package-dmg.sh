#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <binary> <version> <output-directory>" >&2
  exit 64
fi

binary=$1
version=$2
output_directory=$3
staging="$output_directory/staging"
app_bundle="$staging/pollip.app"
output="$output_directory/pollip_${version}_macOS_arm64.dmg"

rm -rf "$staging"
mkdir -p "$app_bundle/Contents/MacOS"
install -m 0755 "$binary" "$app_bundle/Contents/MacOS/pollip"

cat > "$app_bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>pollip</string>
  <key>CFBundleIdentifier</key><string>io.grapefizz.pollip</string>
  <key>CFBundleName</key><string>pollip</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>VERSION</string>
  <key>CFBundleVersion</key><string>VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
</dict></plist>
PLIST
sed -i '' "s/VERSION/$version/g" "$app_bundle/Contents/Info.plist"
ln -s /Applications "$staging/Applications"

hdiutil create -volname pollip -srcfolder "$staging" -ov -format UDZO "$output"
