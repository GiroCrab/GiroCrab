#!/usr/bin/env bash
# Rewrite `version` and `sha256` in the Homebrew cask from a published GitHub release.
#
# Usage: scripts/update-homebrew-cask.sh <TAG> [CASK_FILE]
#   TAG        stable release tag, e.g. v0.5.6 (pre-releases such as "nightly" are refused)
#   CASK_FILE  defaults to packaging/homebrew/Casks/girocrab.rb
#
# Run after `scripts/notarize.sh --upload <TAG>`. Only edits the local file; it never
# commits or pushes. Copy the result into the GiroCrab/homebrew-tap repo (see docs/homebrew.md).
set -euo pipefail

REPO="${GIROCRAB_REPO:-GiroCrab/GiroCrab}"
ASSET="GiroCrab-macOS-arm64.zip"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

TAG="${1:-}"
CASK="${2:-$ROOT/packaging/homebrew/Casks/girocrab.rb}"

[[ "$TAG" =~ ^v[0-9]+(\.[0-9]+)*$ ]] || { echo "usage: $0 vX.Y.Z [CASK_FILE]" >&2; exit 2; }
[ -f "$CASK" ] || { echo "error: cask not found: $CASK" >&2; exit 1; }
VERSION="${TAG#v}"

release="$(gh api "repos/$REPO/releases/tags/$TAG")"
if [ "$(jq -r '.prerelease or .draft' <<<"$release")" = "true" ]; then
  echo "error: $TAG is a draft or pre-release; the cask only tracks stable releases" >&2
  exit 1
fi

digest="$(jq -r --arg n "$ASSET" '.assets[] | select(.name == $n) | .digest // empty' <<<"$release")"
if [[ "$digest" == sha256:* ]]; then
  SHA="${digest#sha256:}"
else
  # Older releases may lack a server-side digest: download and hash.
  url="$(jq -r --arg n "$ASSET" '.assets[] | select(.name == $n) | .browser_download_url // empty' <<<"$release")"
  [ -n "$url" ] || { echo "error: $TAG has no asset $ASSET" >&2; exit 1; }
  tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
  curl -fsSL -o "$tmp/$ASSET" "$url"
  SHA="$(shasum -a 256 "$tmp/$ASSET" | awk '{print $1}')"
fi
[[ "$SHA" =~ ^[0-9a-f]{64}$ ]] || { echo "error: bad sha256 '$SHA'" >&2; exit 1; }

VERSION="$VERSION" SHA="$SHA" perl -0pi -e '
  s/^(\s*version\s+)"[^"]*"/$1"$ENV{VERSION}"/m or die "no version line\n";
  s/^(\s*sha256\s+)"[^"]*"/$1"$ENV{SHA}"/m or die "no sha256 line\n";
' "$CASK"

echo "updated $CASK -> version $VERSION, sha256 $SHA"
echo "next: brew style/audit via local tap, then copy into GiroCrab/homebrew-tap (see docs/homebrew.md)"
