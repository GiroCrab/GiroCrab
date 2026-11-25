#!/usr/bin/env bash
# Payment-path integrity (#1799, #946):
#   1. the central payment files exist,
#   2. .github/CODEOWNERS gives each of them an owner (the last matching rule counts, as on GitHub),
#   3. they contain no bypass patterns (disabled TLS checks, skipped confirmation/validation),
#   4. in a PR (GITHUB_BASE_REF set) that touches them, the security checklist item in the PR
#      body is ticked. Fails closed: an unreachable base or an empty body is an error.
#      Without PR context (local runs) step 4 is skipped.
#
#   scripts/lint-transfer-integrity.sh     exits 1 and lists the problems
#   PR_BODY="$body"                        PR description (set by .github/workflows/pr-checklist.yml)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PAYMENT_FILES=(
  GiroCrabApp/Sources/GiroCrabUI/TransferSheetView.swift
  girocrab-core/src/payment_input.rs
  girocrab-core/src/connectors/fints/sepa.rs
  girocrab-core/src/connectors/fints/transfer.rs
  girocrab-core/src/connectors/fints/debit.rs
  girocrab-core/src/connectors/fints/standing.rs
)
CODEOWNERS=.github/CODEOWNERS
# Payment fields and the Pain.001 builder (#946): a source line naming them is a payment-path
# change wherever it lives (snake_case and camelCase, case-insensitive).
PAYMENT_FIELDS='creditor_?iban|amount_?cents|pain_?\.?001'
# The payment-path item of .github/pull_request_template.md, ticked (keep its text in sync).
CHECKLIST_ITEM='^[[:space:]]*[-*] \[[xX]\] Zahlungspfad geprüft / payment path reviewed'
fail=0
err() { echo "lint-transfer-integrity: $*" >&2; fail=1; }

[[ -f "$CODEOWNERS" ]] || { echo "lint-transfer-integrity: $CODEOWNERS missing" >&2; exit 1; }

# Only anchored patterns (leading `/`, no `**`) are understood below. GitHub applies an
# unanchored rule such as `payment_input.rs` anywhere in the tree; rather than guess, fail closed.
while read -r pat _ || [[ -n "$pat" ]]; do
  [[ -z "$pat" || "$pat" == \#* ]] && continue
  [[ "$pat" == /* && "$pat" != *'**'* ]] || err "unsupported CODEOWNERS pattern (anchor it with a leading /, no **): $pat"
done < "$CODEOWNERS"

# Does CODEOWNERS give the file an owner? Like GitHub, the last matching rule wins; a rule
# without owners (or with only a comment after the pattern) is an ownerless exception.
# Patterns: anchored, optional trailing `/` or `*`.
covered() {
  local f=$1 pat owners owned=1
  while read -r pat owners || [[ -n "$pat" ]]; do
    [[ -z "$pat" || "$pat" == \#* ]] && continue
    pat=${pat#/}
    case "$pat" in
      */) [[ "$f" == "$pat"* ]] || continue ;;
      *) # shellcheck disable=SC2254
         case "$f" in $pat|$pat/*) ;; *) continue ;; esac ;;
    esac
    owners=${owners%%#*}
    if [[ -n "${owners//[[:space:]]/}" ]]; then owned=0; else owned=1; fi
  done < "$CODEOWNERS"
  return "$owned"
}

# Bypass patterns, excluding comment-only lines and test code after `#[cfg(test)]` is not
# distinguished: the patterns must not appear anywhere in these files.
BYPASS='danger_accept_invalid_(certs|hostnames)|accept_invalid_(certs|hostnames)|skip_?(tan|confirm(ation)?|iban_?check|validation)|bypass_?(tan|confirm(ation)?|policy|validation)|\.insecure\(|allowInsecure|trustAll'

for f in "${PAYMENT_FILES[@]}"; do
  if [[ ! -f "$f" ]]; then err "payment file missing: $f"; continue; fi
  covered "$f" || err "$f is not covered by $CODEOWNERS"
  if hits=$(grep -nEi "$BYPASS" "$f"); then err "bypass pattern in $f:"$'\n'"$hits"; fi
done
for p in GiroCrabApp/Sources/GiroCrabUI/BankingPolicy.swift girocrab-core/src/sys.rs .github/CODEOWNERS .github/workflows/x.yml scripts/x.sh policy/x; do
  covered "$p" || err "$p is not covered by $CODEOWNERS"
done

# PR context: payment files touched -> the security checklist item must be ticked.
# Fails closed: a base that cannot be fetched or diffed, or an empty PR body, is an error.
if [[ -n "${GITHUB_BASE_REF:-}" ]]; then
  if ! git fetch -q --no-tags --depth=1 origin "$GITHUB_BASE_REF" 2>/dev/null; then
    err "cannot fetch base '$GITHUB_BASE_REF': the PR's payment-path changes cannot be checked"
  elif ! changed=$(git diff --name-only FETCH_HEAD HEAD); then
    err "cannot diff against base '$GITHUB_BASE_REF': the PR's payment-path changes cannot be checked"
  else
    touched=()
    for f in "${PAYMENT_FILES[@]}"; do
      grep -qxF "$f" <<<"$changed" && touched+=("$f")
    done
    # Added/removed source lines naming payment fields count too (#946). No `grep -q` in the
    # pipe: with `pipefail` an early exit would turn a hit into a miss.
    if ! hunks=$(git diff -U0 FETCH_HEAD HEAD -- '*.rs' '*.swift' '*.kt' '*.qml' '*.slint'); then
      err "cannot diff against base '$GITHUB_BASE_REF': the PR's payment-path changes cannot be checked"
    else
      field_hits=$(grep -Ei "^[+-].*($PAYMENT_FIELDS)" <<<"$hunks" | grep -vE '^(\+\+\+|---) ' || true)
      [[ -n "$field_hits" ]] && touched+=("payment fields ($PAYMENT_FIELDS)")
    fi
    if (( ${#touched[@]} )); then
      echo "lint-transfer-integrity: PR touches the payment path: ${touched[*]}"
      if [[ -z "${PR_BODY:-}" ]]; then
        err "PR touches the payment path but has no description: add and tick the payment-path security checklist item"
      elif ! grep -qE "$CHECKLIST_ITEM" <<<"$PR_BODY"; then
        err "PR touches the payment path: tick the payment-path security checklist item in the PR description"
      fi
    fi
  fi
fi

(( fail )) && exit 1
echo "lint-transfer-integrity: ok"
