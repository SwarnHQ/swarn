#!/bin/sh
# Builds a release Swarn.app and packs it into target/release/Swarn-<version>.dmg, with a
# shortcut to /Applications to drag it onto. Also writes a .sha256 next to it for install.sh.
#
# Unsigned by default, which is fine for trying it on your own Mac. For a build other people can
# open without a Gatekeeper warning, set both:
#   SWARN_SIGN_ID          a "Developer ID Application: ..." identity in your keychain
#   SWARN_NOTARY_PROFILE   a profile saved with `xcrun notarytool store-credentials`
set -eu
cd "$(dirname "$0")/.."

APP=$(scripts/bundle.sh release | tail -1)
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/swarn-desktop/Cargo.toml | head -1)
DMG="target/release/Swarn-$VERSION.dmg"

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -quiet -volname "Swarn $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO "$DMG"

if [ -n "${SWARN_SIGN_ID:-}" ]; then
  codesign --force --timestamp --sign "$SWARN_SIGN_ID" "$DMG"
fi
if [ -n "${SWARN_NOTARY_PROFILE:-}" ]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$SWARN_NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
fi

(cd "$(dirname "$DMG")" && shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256")
echo "$DMG"
