#!/usr/bin/env bash
# Relative links and anchors in tracked Markdown files lead somewhere (#1875),
# checked offline with lychee (no network requests).
# Skipped: external URLs (--offline), Markdown line anchors (`#L42`), `../../releases`
# (resolves only on github.com), `0000-template.md`, `cashcrab-core/` and `CashCrabApp/`.
set -euo pipefail

if ! command -v lychee >/dev/null; then
  echo "check-doc-links: lychee not found (install e.g. 'brew install lychee' or 'cargo install lychee')" >&2
  exit 127
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$ROOT"
git rev-parse --is-inside-work-tree >/dev/null

# `../../releases` in README.md resolves two levels above the repo root on github.com.
releases_re="$(python3 -I - "$ROOT/../.." <<'PY'
import re, sys
from pathlib import Path
print("^" + re.escape((Path(sys.argv[1]).resolve() / "releases").as_uri()) + "$")
PY
)"

files=()
while IFS= read -r -d '' f; do
  case "$f" in
    cashcrab-core/*|CashCrabApp/*|0000-template.md|*/0000-template.md) continue ;;
  esac
  [ -e "$f" ] || continue
  # Escape glob metacharacters so lychee treats tracked filenames literally.
  case "$f" in
    *[][*?]* ) f="$(printf '%s\n' "$f" | sed 's/[][*?]/[&]/g')" ;;
  esac
  files+=("$f")
done < <(git ls-files -z ':(icase)*.md')

[ "${#files[@]}" -eq 0 ] && exit 0

exec lychee \
  --offline \
  --include-fragments \
  --root-dir "$ROOT" \
  --no-progress \
  --exclude "$releases_re" \
  --exclude '\.[mM][dD]#L[0-9]+(C[0-9]+)?(-L[0-9]+(C[0-9]+)?)?$' \
  -- "${files[@]}"
