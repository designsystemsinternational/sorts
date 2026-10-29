#!/usr/bin/env bash
# Releases a new version of sorts: bumps the version, runs all checks, commits, tags,
# pushes, and updates the Homebrew formula in the tap.
#
# Usage:
#   scripts/release.sh
#
# The script asks before every step that changes something on GitHub.
#
# Environment:
#   TAP_DIR                    the tap checkout. Default: ../homebrew-tap next to this repo.
#   SORTS_RELEASE_SOURCE_URL   for tests only: the source archive URL for the formula.

set -euo pipefail

repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
tap_dir="${TAP_DIR:-$repo_dir/../homebrew-tap}"
cd "$repo_dir"

fail() {
  echo "Error: $*" >&2
  exit 1
}

confirm() {
  local answer
  read -r -p "$1 [y/N] " answer
  [[ "$answer" == "y" || "$answer" == "yes" ]]
}

# --- Preconditions ---------------------------------------------------------

[[ "$(git branch --show-current)" == "main" ]] || fail "Switch to the main branch first."
[[ -z "$(git status --porcelain)" ]] || fail "The working tree has changes. Commit or stash them first."

git fetch --quiet origin
if git rev-parse --verify --quiet origin/main >/dev/null; then
  [[ -z "$(git log --oneline HEAD..origin/main)" ]] || fail "origin/main has commits that you do not have. Pull first."
fi

[[ -d "$tap_dir/.git" ]] || fail "No tap checkout at $tap_dir. Clone homebrew-tap there or set TAP_DIR."
[[ -z "$(git -C "$tap_dir" status --porcelain)" ]] || fail "The tap checkout at $tap_dir has changes."

# --- Choose the version ----------------------------------------------------

current="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
IFS=. read -r major minor patch <<<"$current"
next_patch="$major.$minor.$((patch + 1))"
next_minor="$major.$((minor + 1)).0"
next_major="$((major + 1)).0.0"

echo "Current version: $current"
echo
echo "Changes since v$current:"
git log --oneline "v$current..HEAD" 2>/dev/null | sed 's/^/  /' || true
echo
echo "  1) patch  $next_patch  bug fixes only"
echo "  2) minor  $next_minor  new features"
echo "  3) major  $next_major  changes that break how people use it"
read -r -p "Choose 1, 2, or 3: " choice
case "$choice" in
  1) version="$next_patch" ;;
  2) version="$next_minor" ;;
  3) version="$next_major" ;;
  *) fail "No version chosen." ;;
esac
tag="v$version"
git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null && fail "The tag $tag already exists."

# --- Bump and check --------------------------------------------------------

awk -v v="$version" '!done && /^version = / { $0 = "version = \"" v "\""; done = 1 } { print }' \
  Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml

echo
echo "Running all checks for $version ..."
if ! (cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo test -- --ignored); then
  git checkout -- Cargo.toml Cargo.lock
  fail "The checks failed. The version is not changed."
fi
[[ "$(cargo run --quiet -- --version)" == *"$version"* ]] || fail "sorts --version does not show $version."

git add Cargo.toml Cargo.lock
git commit --quiet -m "Release $tag"
git tag "$tag"
echo
echo "Committed and tagged $tag locally."

# --- Push ------------------------------------------------------------------

if ! confirm "Push main and $tag to origin?"; then
  echo "Nothing is pushed. To undo the local release:"
  echo "  git tag -d $tag && git reset --hard HEAD~1"
  exit 0
fi
git push --quiet origin main "$tag"
echo "Pushed main and $tag."

# --- Homebrew formula ------------------------------------------------------

git -C "$tap_dir" pull --quiet --ff-only
"$repo_dir/scripts/update-formula.sh" "$version" "$tap_dir" ${SORTS_RELEASE_SOURCE_URL:+"$SORTS_RELEASE_SOURCE_URL"}
if command -v brew >/dev/null; then
  # `brew style` turns on developer mode. Turn it off again if it was off.
  developer_was_off="$(brew developer | grep -q disabled && echo yes || true)"
  brew style "$tap_dir/Formula/sorts.rb"
  if [[ -n "$developer_was_off" ]]; then
    brew developer off >/dev/null
  fi
fi
if git -C "$tap_dir" diff --quiet; then
  echo "The formula in the tap is already up to date."
  exit 0
fi
git -C "$tap_dir" --no-pager diff
if ! confirm "Commit and push the formula for $version to the tap?"; then
  echo "The tap is not pushed. The changed formula is in $tap_dir/Formula/sorts.rb."
  exit 0
fi
git -C "$tap_dir" add Formula/sorts.rb
git -C "$tap_dir" commit --quiet -m "sorts $version"
git -C "$tap_dir" push --quiet
echo
echo "Released sorts $version."
echo "Team members update with: brew update && brew upgrade sorts"
