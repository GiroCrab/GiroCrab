#!/usr/bin/env bash
# GiroCrabUI is also built for iOS. CI has no iOS Simulator job (too slow), so macOS-only
# APIs are caught here by grep instead: a use outside `#if os(macOS)` fails.
#
#   scripts/lint-ios-compat.sh      exits 1 and lists offending lines
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# macOS-only APIs (unavailable on iOS). Extend when another one bites.
PATTERN='homeDirectoryForCurrentUser|NSWorkspace\.shared|NSOpenPanel|NSSavePanel|toggleStyle[(][[:space:]]*[.]checkbox[[:space:]]*[)]|buttonStyle[(][[:space:]]*[.]link[[:space:]]*[)]'

# A branch counts as macOS-only if its condition is exactly os(macOS) or !os(iOS). GiroCrabUI
# builds for macOS and iOS only, so a branch after an iOS-only one (exactly `#if os(iOS)` /
# `#if !os(macOS)`, then `#elseif` / `#else`) is macOS-only too, and the `#else` of `#if os(macOS)`
# is checked. Any other condition (`&&`, `||`, parentheses) leaves its branches checked.
bad=$(find GiroCrabApp/Sources/GiroCrabUI -name '*.swift' -print0 | xargs -0 awk -v pat="$PATTERN" '
    function cond(l) {
        sub(/^[[:space:]]*#(else)?if[[:space:]]+/, "", l); sub(/[[:space:]]*\/\/.*$/, "", l); gsub(/[[:space:]]/, "", l)
        return l
    }
    function maconly(c) { c = cond(c); return c == "os(macOS)" || c == "!os(iOS)" }
    function iosonly(c) { c = cond(c); return c == "os(iOS)" || c == "!os(macOS)" }
    FNR == 1 { depth = 0 }
    /^[[:space:]]*#if[[:space:]]/ { depth++; m[depth] = maconly($0); ios[depth] = iosonly($0); next }
    /^[[:space:]]*#elseif[[:space:]]/ { m[depth] = ios[depth] || maconly($0); ios[depth] = ios[depth] || iosonly($0); next }
    /^[[:space:]]*#else/ { m[depth] = ios[depth]; next }
    /^[[:space:]]*#endif/ { depth--; next }
    {
        mac = 0
        for (i = 1; i <= depth; i++) if (m[i]) mac = 1
        if (!mac && $0 !~ /^[[:space:]]*\/\// && $0 ~ pat) print FILENAME ":" FNR ": " $0
    }
')
if [[ -n "$bad" ]]; then
    echo "macOS-only API outside #if os(macOS) (breaks the iOS build):" >&2
    echo "$bad" >&2
    exit 1
fi
