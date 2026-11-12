#!/usr/bin/env bash
# Rewrite `version` and `sha256` in a Homebrew cask from a published GitHub release.
#
# Usage: scripts/update-homebrew-cask.sh <TAG> [CASK_FILE]
#   TAG        vX.Y.Z         stable release   -> packaging/homebrew/Casks/girocrab.rb
#              vX.Y.Z-beta.N  beta pre-release -> packaging/homebrew/Casks/girocrab@beta.rb
#              ("nightly" is refused: girocrab@nightly uses version :latest, no checksum)
#   CASK_FILE  overrides the default cask file
#
# Run after `scripts/notarize.sh --upload <TAG>`. Only edits the local file; it never
# commits or pushes. Copy the result into the GiroCrab/homebrew-tap repo (see docs/homebrew.md).
set -euo pipefail

REPO="${GIROCRAB_REPO:-GiroCrab/GiroCrab}"
ASSET="GiroCrab-macOS-arm64.zip"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

TAG="${1:-}"
if [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  KIND="stable"; DEFAULT_CASK="girocrab.rb"
elif [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+-beta\.[0-9]+$ ]]; then
  KIND="beta"; DEFAULT_CASK="girocrab@beta.rb"
else
  echo "usage: $0 vX.Y.Z|vX.Y.Z-beta.N [CASK_FILE]" >&2; exit 2
fi
CASK="${2:-$ROOT/packaging/homebrew/Casks/$DEFAULT_CASK}"
[ -f "$CASK" ] || { echo "error: cask not found: $CASK" >&2; exit 1; }
VERSION="${TAG#v}"

release="$(gh api "repos/$REPO/releases/tags/$TAG")"
if [ "$(jq -r '.draft' <<<"$release")" = "true" ]; then
  echo "error: $TAG is still a draft; publish it first" >&2
  exit 1
fi
PRE="$(jq -r '.prerelease' <<<"$release")"
if [ "$KIND" = "stable" ] && [ "$PRE" = "true" ]; then
  echo "error: $TAG is a pre-release; the stable cask only tracks stable releases" >&2
  exit 1
fi
if [ "$KIND" = "beta" ] && [ "$PRE" != "true" ]; then
  echo "error: $TAG is not marked as pre-release; betas must be pre-releases" >&2
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
