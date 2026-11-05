#!/usr/bin/env bash
# Build ArchGuard.app for macOS: the desktop app with its icon, plus the
# `archguard` CLI next to it (the app starts `archguard serve` for the web
# viewer fallback). Uses only Apple's tools (sips, iconutil, plutil).
#
# Usage: scripts/desktop/bundle-macos.sh [out-dir]   (default: target/bundle)
# Not signed or notarised yet (phase 8 of ADR 0002).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:-$root/target/bundle}"
app="$out/ArchGuard.app"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"

cargo build --release --manifest-path "$root/Cargo.toml" -p archguard-desktop -p archguard-cli

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/archguard-desktop" "$app/Contents/MacOS/"
cp "$root/target/release/archguard" "$app/Contents/MacOS/"

# Icon set from the 1024 px master (assets/icon.png, drawn from icon.svg).
iconset="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$iconset"
master="$root/crates/archguard-desktop/assets/icon.png"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$master" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$master" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>ArchGuard</string>
  <key>CFBundleDisplayName</key><string>ArchGuard</string>
  <key>CFBundleIdentifier</key><string>com.github.svenhartmann.archguard</string>
  <key>CFBundleExecutable</key><string>archguard-desktop</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null

echo "$app"
