#!/usr/bin/env bash
# Writes Formula/sorts.rb in a checkout of the Homebrew tap for a released version.
#
# Usage:
#   scripts/update-formula.sh <version> <tap-dir> [<source-url>]
#
# <version>    e.g. 0.1.0. The tag v<version> must already be pushed to GitHub.
# <tap-dir>    a local checkout of designsystemsinternational/homebrew-tap.
# <source-url> optional, for tests. Defaults to the GitHub archive of the tag.
#
# The script only writes the formula file. Commit and push the tap yourself.

set -euo pipefail

if [[ $# -lt 2 ]]; then
  sed -n '2,12p' "$0"
  exit 1
fi

version="$1"
tap_dir="$2"
url="${3:-https://github.com/designsystemsinternational/sorts/archive/refs/tags/v${version}.tar.gz}"
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"

cargo_version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo_dir/Cargo.toml" | head -1)"
if [[ "$cargo_version" != "$version" ]]; then
  echo "Error: Cargo.toml has version $cargo_version, not $version." >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl --fail --silent --show-error --location "$url" --output "$tmp/source.tar.gz"
sha256="$(shasum -a 256 "$tmp/source.tar.gz" | cut -d' ' -f1)"

mkdir -p "$tap_dir/Formula"
sed -e "s|{{URL}}|$url|" -e "s|{{SHA256}}|$sha256|" \
  "$repo_dir/packaging/homebrew/sorts.rb.template" > "$tap_dir/Formula/sorts.rb"

echo "Wrote $tap_dir/Formula/sorts.rb"
echo "  url:    $url"
echo "  sha256: $sha256"
