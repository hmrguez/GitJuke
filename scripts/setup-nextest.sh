#!/usr/bin/env bash
# Install the reviewed, checksum-pinned runner inside the ignored target tree.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
version=0.9.148
destination="$root/target/test-tools"
if [[ -x "$destination/cargo-nextest" ]] && \
   "$destination/cargo-nextest" --version | head -n 1 | grep -Fq "cargo-nextest $version "; then
  exit 0
fi
if [[ "$(uname -s)" != Darwin ]]; then
  echo "GitJuke's pinned test runner currently targets macOS (Intel and Apple Silicon)." >&2
  exit 1
fi
archive="$(mktemp -t gitjuke-nextest)"
trap 'rm -f "$archive"' EXIT
curl --fail --location --silent --show-error --retry 3 \
  "https://get.nexte.st/$version/mac" --output "$archive"
echo "6c23b7fb4ca82c571fc8dfedbab45bfa773d45918f933170c85d21cd7f4b852b  $archive" | shasum -a 256 --check
mkdir -p "$destination"
tar -xzf "$archive" -C "$destination" cargo-nextest
"$destination/cargo-nextest" --version
