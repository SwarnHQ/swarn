#!/bin/sh
# Builds Swarn.app with the daemon inside it, ready for SMAppService to register (PRD §5.5).
# Usage: scripts/bundle.sh [profile]   (default: release). Signs ad hoc; Developer ID comes in week 8.
set -eu
PROFILE="${1:-release}"
DIR="target/$PROFILE"
[ "$PROFILE" = "dev" ] && DIR="target/debug"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/swarn-desktop/Cargo.toml | head -1)

cargo build --profile "$PROFILE" -p swarn-desktop -p swarn-daemon

APP="$DIR/Swarn.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Library/LaunchAgents"
cp "$DIR/swarn" "$APP/Contents/MacOS/Swarn"
cp "$DIR/swarnd" "$APP/Contents/MacOS/swarnd"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.swarn.desktop</string>
  <key>CFBundleName</key><string>Swarn</string>
  <key>CFBundleDisplayName</key><string>Swarn</string>
  <key>CFBundleExecutable</key><string>Swarn</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST

# KeepAlive restarts the daemon if it crashes, not after a clean exit. AssociatedBundleIdentifiers
# makes Login Items show it as part of Swarn rather than as an anonymous binary.
cat > "$APP/Contents/Library/LaunchAgents/dev.swarn.daemon.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>dev.swarn.daemon</string>
  <key>BundleProgram</key><string>Contents/MacOS/swarnd</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Standard</string>
  <key>AssociatedBundleIdentifiers</key><array><string>dev.swarn.desktop</string></array>
</dict></plist>
PLIST

codesign --force --deep --sign - "$APP"
echo "$APP"
