#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
test "$(uname -m)" = arm64
profile="${1:-release}"
version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' packaging/macos/Info.plist)"
export MACOSX_DEPLOYMENT_TARGET=15.0
if [[ "$profile" == release ]]; then cargo build --locked --release --target aarch64-apple-darwin; else cargo build --locked --target aarch64-apple-darwin; fi
bundle="dist/Simple Snip.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources" dist/AppIcon.iconset
cp "target/aarch64-apple-darwin/$profile/simple-snip" "$bundle/Contents/MacOS/"
cp packaging/macos/Info.plist "$bundle/Contents/Info.plist"
for dimension in 16 32 128 256 512; do
  sips -z "$dimension" "$dimension" assets/icons/app.png --out "dist/AppIcon.iconset/icon_${dimension}x${dimension}.png" >/dev/null
  double=$((dimension*2))
  sips -z "$double" "$double" assets/icons/app.png --out "dist/AppIcon.iconset/icon_${dimension}x${dimension}@2x.png" >/dev/null
done
iconutil -c icns dist/AppIcon.iconset -o "$bundle/Contents/Resources/AppIcon.icns"
cp LICENSE README.md "$bundle/Contents/Resources/"
if [[ -f dist/THIRD_PARTY_NOTICES.txt ]]; then cp dist/THIRD_PARTY_NOTICES.txt "$bundle/Contents/Resources/"; fi
# Local test signature only; never claim Developer ID or notarization.
if [[ -n "${SIMPLE_SNIP_SIGN_IDENTITY:-}" ]]; then
  codesign --force --options runtime --timestamp --sign "$SIMPLE_SNIP_SIGN_IDENTITY" "$bundle"
else
  echo "LOCAL TEST: ad-hoc signatures change with the executable; macOS may require adding this app again in Screen Recording settings."
  codesign --force --deep --sign - "$bundle"
fi
codesign --verify --deep --strict "$bundle"
if [[ "$profile" == release ]]; then
  ditto -c -k --keepParent "$bundle" "dist/SimpleSnip_${version}_macos-arm64_unsigned.zip"
  hdiutil create -volname "Simple Snip (unsigned)" -srcfolder "$bundle" -ov -format UDZO "dist/SimpleSnip_${version}_macos-arm64_unsigned.dmg"
fi
