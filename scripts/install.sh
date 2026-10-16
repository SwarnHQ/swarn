#!/bin/sh
# Installs the latest Swarn release into /Applications (or ~/Applications if that isn't writable).
#
#   curl -fsSL https://raw.githubusercontent.com/SwarnHQ/swarn/main/scripts/install.sh | sh
#
# It downloads the DMG from GitHub Releases, checks its SHA-256, and copies Swarn.app out of it.
# Nothing else on your Mac is touched. Overrides, mostly for testing:
#   SWARN_DOWNLOAD     where to fetch from (default: the latest GitHub release)
#   SWARN_INSTALL_DIR  where to put Swarn.app
set -eu

ASSET="Swarn-macos-arm64.dmg"
BASE="${SWARN_DOWNLOAD:-https://github.com/SwarnHQ/swarn/releases/latest/download}"

fail() { echo "swarn install: $*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || fail "Swarn runs on macOS only."
[ "$(uname -m)" = arm64 ] || fail "Swarn needs a Mac with Apple silicon."
pgrep -xq Swarn && fail "Swarn is running. Quit it first, then run this again."

if [ -n "${SWARN_INSTALL_DIR:-}" ]; then
  DEST="$SWARN_INSTALL_DIR"
elif [ -w /Applications ]; then
  DEST=/Applications
else
  DEST="$HOME/Applications"
fi
mkdir -p "$DEST"

WORK=$(mktemp -d)
MOUNT="$WORK/mount"
cleanup() {
  hdiutil detach -quiet "$MOUNT" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "Downloading $ASSET"
curl -fL --progress-bar -o "$WORK/$ASSET" "$BASE/$ASSET"
curl -fsSL -o "$WORK/$ASSET.sha256" "$BASE/$ASSET.sha256"
(cd "$WORK" && shasum -a 256 -c "$ASSET.sha256" >/dev/null) \
  || fail "the download doesn't match its checksum. Nothing was installed."

mkdir "$MOUNT"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$MOUNT" "$WORK/$ASSET"
[ -d "$MOUNT/Swarn.app" ] || fail "the disk image has no Swarn.app in it."

# Replace an older copy only if it really is Swarn, so a typo in SWARN_INSTALL_DIR can't delete
# some other app.
if [ -e "$DEST/Swarn.app" ]; then
  ID=$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$DEST/Swarn.app/Contents/Info.plist" 2>/dev/null || true)
  [ "$ID" = dev.swarn.desktop ] || fail "$DEST/Swarn.app exists and isn't Swarn. Move it away first."
  rm -rf "$DEST/Swarn.app"
fi
ditto "$MOUNT/Swarn.app" "$DEST/Swarn.app"

VERSION=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$DEST/Swarn.app/Contents/Info.plist")
echo "Installed Swarn $VERSION in $DEST."
echo "Open it with: open \"$DEST/Swarn.app\""
