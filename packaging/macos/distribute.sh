#!/bin/bash
# A signed release is published only after both the app and DMG pass Gatekeeper.
set -euo pipefail
bundle="$1"
version="$2"
output="$3"
: "${SIMPLE_SNIP_SIGN_IDENTITY:?Set a Developer ID Application signing identity}"
: "${SIMPLE_SNIP_NOTARY_PROFILE:?Store notarytool credentials and set their profile name}"
mkdir -p "$output"
staging="$(mktemp -d "$output/.notarize.XXXXXX")"
trap 'distribution_status=$?; rm -rf "$staging"; exit "$distribution_status"' EXIT
auth=(--keychain-profile "$SIMPLE_SNIP_NOTARY_PROFILE")
signing=(codesign)
if [[ -n "${SIMPLE_SNIP_KEYCHAIN:-}" ]]; then
  auth+=(--keychain "$SIMPLE_SNIP_KEYCHAIN")
  signing+=(--keychain "$SIMPLE_SNIP_KEYCHAIN")
fi
notarize() {
  local archive="$1" result="$staging/result.json"
  xcrun notarytool submit "$archive" "${auth[@]}" --wait --timeout 30m --output-format json > "$result"
  # A successful upload is not an accepted notarization. Require the final state.
  if ! python3 - "$result" <<'PY'
import json, sys
result = json.load(open(sys.argv[1]))
if result.get("status") != "Accepted":
    print("Apple notarization was not accepted: " + str(result.get("status")), file=sys.stderr)
    sys.exit(1)
PY
  then
    local submission
    submission="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$result")"
    xcrun notarytool log "$submission" "${auth[@]}" || true
    return 1
  fi
}
"${signing[@]}" --force --options runtime --timestamp --sign "$SIMPLE_SNIP_SIGN_IDENTITY" "$bundle"
codesign --verify --deep --strict "$bundle"
if ! codesign --display --verbose=4 "$bundle" 2>&1 | /usr/bin/grep -q '^Authority=Developer ID Application:'; then
  echo "Release signing requires Developer ID Application, not ad-hoc or Apple Development." >&2
  exit 1
fi
ditto -c -k --keepParent "$bundle" "$staging/submission.zip"
notarize "$staging/submission.zip"
xcrun stapler staple "$bundle"
xcrun stapler validate "$bundle"
spctl --assess --type execute --verbose=2 "$bundle"
name="SimpleSnip_${version}_macos-arm64"
# ZIPs cannot be stapled; re-archive the app after its ticket is attached.
ditto -c -k --keepParent "$bundle" "$staging/$name.zip"
hdiutil create -volname "Simple Snip" -srcfolder "$bundle" -ov -format UDZO "$staging/$name.dmg"
"${signing[@]}" --force --timestamp --sign "$SIMPLE_SNIP_SIGN_IDENTITY" "$staging/$name.dmg"
notarize "$staging/$name.dmg"
xcrun stapler staple "$staging/$name.dmg"
xcrun stapler validate "$staging/$name.dmg"
spctl --assess --type open --context context:primary-signature --verbose=2 "$staging/$name.dmg"
mv "$staging/$name.zip" "$staging/$name.dmg" "$output/"
echo "Developer ID signed, notarized and stapled: $output/$name.{zip,dmg}"
