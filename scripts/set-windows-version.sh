#!/usr/bin/env bash
# Sets the version of GiroCrabWindows (Cargo.toml + the package entry in Cargo.lock) to a release tag's
# version, e.g. v0.9.0-nightly.20261124 -> 0.9.0-nightly.20261124 (#1778). updates.rs reads it at compile time.
#   scripts/set-windows-version.sh <tag>
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
TAG="${1:?usage: $0 <tag>}"
VER="${TAG#v}"
[[ "$VER" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-beta\.[0-9]+|-nightly\.[0-9]{8})?$ ]] \
  || { echo "error: '$TAG' is not vX.Y.Z[-beta.N|-nightly.YYYYMMDD]" >&2; exit 2; }
python3 -I - "$VER" <<'PY'
import re, sys
ver = sys.argv[1]
def patch(path, pattern):
    s = open(path, encoding="utf-8").read()
    s, n = re.subn(pattern, lambda m: m.group(1) + ver + m.group(3), s, count=1, flags=re.M)
    if n != 1:
        sys.exit(f"error: version line not found in {path}")
    open(path, "w", encoding="utf-8").write(s)
patch("GiroCrabWindows/Cargo.toml", r'^(version = ")([^"]+)(")')
patch("GiroCrabWindows/Cargo.lock", r'(^name = "girocrab-windows"\nversion = ")([^"]+)(")')
PY
grep -m1 '^version' GiroCrabWindows/Cargo.toml
