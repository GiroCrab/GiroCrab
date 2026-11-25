#!/usr/bin/env bash
# Payment-path integrity (#1799, #946):
#   1. the central payment files exist,
#   2. .github/CODEOWNERS covers each of them,
#   3. they contain no bypass patterns (disabled TLS checks, skipped confirmation/validation),
#   4. in a PR (GITHUB_BASE_REF set) that touches them, the security checklist in the PR body
#      is ticked. Without PR context (local runs) this step only prints a note and never fails.
#
#   scripts/lint-transfer-integrity.sh     exits 1 and lists the problems
#   PR_BODY="$body"                        PR description (set by ci.yml); unset = cannot check
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PAYMENT_FILES=(
  GiroCrabApp/Sources/GiroCrabUI/TransferSheetView.swift
  girocrab-core/src/payment_input.rs
  girocrab-core/src/connectors/fints/sepa.rs
  girocrab-core/src/connectors/fints/transfer.rs
)
CODEOWNERS=.github/CODEOWNERS
fail=0
err() { echo "lint-transfer-integrity: $*" >&2; fail=1; }

[[ -f "$CODEOWNERS" ]] || { echo "lint-transfer-integrity: $CODEOWNERS missing" >&2; exit 1; }

# Does a CODEOWNERS pattern (anchored, optional trailing `/` or `*`) cover the file?
covered() {
  local f=$1 pat
  while read -r pat _; do
    [[ -z "$pat" || "$pat" == \#* ]] && continue
    pat=${pat#/}
    case "$pat" in
      */) [[ "$f" == "$pat"* ]] && return 0 ;;
      *) # shellcheck disable=SC2254
         case "$f" in $pat|$pat/*) return 0 ;; esac ;;
    esac
  done < "$CODEOWNERS"
  return 1
}

# Bypass patterns, excluding comment-only lines and test code after `#[cfg(test)]` is not
# distinguished: the patterns must not appear anywhere in these files.
BYPASS='danger_accept_invalid_(certs|hostnames)|accept_invalid_(certs|hostnames)|skip_?(tan|confirm(ation)?|iban_?check|validation)|bypass_?(tan|confirm(ation)?|policy|validation)|\.insecure\(|allowInsecure|trustAll'

for f in "${PAYMENT_FILES[@]}"; do
  if [[ ! -f "$f" ]]; then err "payment file missing: $f"; continue; fi
  covered "$f" || err "$f is not covered by $CODEOWNERS"
  if hits=$(grep -nEi "$BYPASS" "$f"); then err "bypass pattern in $f:"$'\n'"$hits"; fi
done
for p in GiroCrabApp/Sources/GiroCrabUI/BankingPolicy.swift girocrab-core/src/sys.rs .github/workflows/x.yml scripts/x.sh policy/x; do
  covered "$p" || err "$p is not covered by $CODEOWNERS"
done

# PR context: payment files touched -> security checklist must be ticked.
if [[ -n "${GITHUB_BASE_REF:-}" ]]; then
  changed=""
  if git fetch -q --no-tags --depth=1 origin "$GITHUB_BASE_REF" 2>/dev/null; then
    changed=$(git diff --name-only FETCH_HEAD HEAD 2>/dev/null || true)
  else
    echo "lint-transfer-integrity: note: cannot fetch base '$GITHUB_BASE_REF', PR checklist not checked"
  fi
  touched=()
  for f in "${PAYMENT_FILES[@]}"; do
    grep -qxF "$f" <<<"$changed" && touched+=("$f")
  done
  if (( ${#touched[@]} )); then
    echo "lint-transfer-integrity: PR touches the payment path: ${touched[*]}"
    if [[ -z "${PR_BODY:-}" ]]; then
      echo "lint-transfer-integrity: note: PR_BODY not set, cannot check the security checklist"
    elif ! grep -qiE '^\s*[-*] \[x\].*(Zahlungs|payment)' <<<"$PR_BODY"; then
      err "PR touches the payment path: tick the payment-path security checklist item in the PR description"
    fi
  fi
fi

(( fail )) && exit 1
echo "lint-transfer-integrity: ok"
