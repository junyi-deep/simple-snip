#!/bin/bash
# Run only on the disposable GitHub Actions macOS runner, never a developer Mac.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == true ]] || { echo "CI runner only" >&2; exit 1; }
: "${MACOS_CERTIFICATE_P12_BASE64:?Missing Developer ID certificate secret}"
: "${MACOS_CERTIFICATE_PASSWORD:?Missing certificate password secret}"
: "${MACOS_SIGN_IDENTITY:?Missing Developer ID Application identity secret}"
: "${APPLE_ID:?Missing notarization Apple ID secret}"
: "${APPLE_TEAM_ID:?Missing Apple team secret}"
: "${APPLE_APP_PASSWORD:?Missing app-specific password secret}"
umask 077
certificate="$(mktemp "$RUNNER_TEMP/simple-snip-certificate.XXXXXX")"
trap 'rm -f "$certificate"' EXIT
printf '%s' "$MACOS_CERTIFICATE_P12_BASE64" | base64 --decode > "$certificate"
keychain="$RUNNER_TEMP/simple-snip-signing.keychain-db"
password="$(openssl rand -base64 32)"
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$certificate" -P "$MACOS_CERTIFICATE_PASSWORD" -k "$keychain" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
xcrun notarytool store-credentials simple-snip-notary --keychain "$keychain" \
  --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD"
{
  printf 'SIMPLE_SNIP_KEYCHAIN=%s\n' "$keychain"
  printf 'SIMPLE_SNIP_SIGN_IDENTITY=%s\n' "$MACOS_SIGN_IDENTITY"
  printf 'SIMPLE_SNIP_NOTARY_PROFILE=simple-snip-notary\n'
} >> "$GITHUB_ENV"
