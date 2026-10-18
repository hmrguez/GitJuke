#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != Darwin ]]; then
  echo "GitJuke requires macOS to build." >&2
  exit 1
fi

mode="${1:-local}"
case "$mode" in
  local)
    # A personal build never submits anything to Apple, even if credentials exist.
    unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_PATH
    unset APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD
    export APPLE_SIGNING_IDENTITY="-"
    bundles=app
    ;;
  signed)
    : "${APPLE_SIGNING_IDENTITY:?Set your certificate identity from security find-identity -v -p codesigning}"
    bundles=app,dmg
    ;;
  release)
    : "${APPLE_SIGNING_IDENTITY:?Missing Developer ID Application signing identity}"
    : "${APPLE_ID:?Missing notarization Apple ID}"
    : "${APPLE_PASSWORD:?Missing notarization app-specific password}"
    : "${APPLE_TEAM_ID:?Missing Apple team ID}"
    if [[ "$APPLE_SIGNING_IDENTITY" != 'Developer ID Application: '* ]]; then
      echo "Official releases require a Developer ID Application identity." >&2
      exit 1
    fi
    # Tauri removes the intermediate .app when only a DMG is requested.
    # Retain it so signature, stapling, and Gatekeeper checks inspect the app.
    bundles=app,dmg
    ;;
  *) echo "Usage: bash scripts/build-desktop.sh [local|signed|release]" >&2; exit 1 ;;
esac

# Honor rust-toolchain.toml even when Homebrew's cargo/rustc precede rustup in PATH.
cargo_path="$(rustup which cargo)"
export PATH="$(dirname "$cargo_path"):$PATH"
bun scripts/check-release.ts
bun run tauri build --ci --target aarch64-apple-darwin --bundles "$bundles" -- --locked

app=target/aarch64-apple-darwin/release/bundle/macos/GitJuke.app
codesign --verify --deep --strict --verbose=2 "$app"
if [[ "$mode" == release ]]; then
  # Fail closed if signing, notarization, or stapling did not succeed.
  codesign --display --verbose=4 "$app" 2>&1 | grep -Fx "TeamIdentifier=$APPLE_TEAM_ID"
  xcrun stapler validate "$app"
  spctl --assess --type execute --verbose=2 "$app"
fi
