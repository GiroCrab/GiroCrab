#!/usr/bin/env bash
# The schema steps in girocrab-core/src/schema.rs and their descriptions stay in sync:
# every step 1..N of `STEPS` has a line in the module comment of schema.rs and a row in
# the table of docs/ARCHITECTURE.md, and the table's "future changes" row is "N+1+".
set -euo pipefail
cd "$(dirname "$0")/.."
schema=girocrab-core/src/schema.rs
doc=docs/ARCHITECTURE.md

# One `Step::…` entry per line between `const STEPS` and its closing `];` (rustfmt keeps it so).
n=$(awk '/^const STEPS: /{on=1; next} on && /^\];/{exit} on && /^[[:space:]]*Step::/{c++} END{print c+0}' "$schema")
if [ "$n" -lt 1 ]; then
    echo "$schema: no entries found in STEPS (did its layout change? update $0)" >&2
    exit 1
fi

# Prints the step numbers a list of keys covers ("12", "2–8", "2-8"), one per line;
# a key ending in "+" is printed as "<number>+".
expand() {
    sed 's/–/-/g' | awk '{
        k = $0
        if (k ~ /^[0-9]+\+$/) { print k; next }
        split(k, r, /-/)
        hi = (r[2] == "") ? r[1] : r[2]
        for (i = r[1]; i <= hi; i++) print i
    }'
}

# Reports steps 1..N missing from a source and numbers beyond N.
check() { # name, keys (one per line)
    local name=$1 keys=$2 errs=0 i
    for i in $(seq 1 "$n"); do
        grep -qx "$i" <<<"$keys" || { echo "$name: step $i of $n is not described" >&2; errs=1; }
    done
    while read -r k; do
        [ -z "$k" ] && continue
        case $k in
            *+) [ "${k%+}" = "$((n + 1))" ] || { echo "$name: row \"$k\" should read \"$((n + 1))+\"" >&2; errs=1; } ;;
            *) [ "$k" -le "$n" ] || { echo "$name: describes step $k, but STEPS has only $n" >&2; errs=1; } ;;
        esac
    done <<<"$keys"
    return $errs
}

fail=0
module_keys=$(sed -n 's/^\/\/! - \([0-9][0-9–-]*\):.*/\1/p' "$schema" | expand)
check "$schema (module comment)" "$module_keys" || fail=1

table_keys=$(sed -n 's/^| *\([0-9][0-9–+-]*\) *|.*/\1/p' "$doc" | expand)
check "$doc (schema table)" "$table_keys" || fail=1
grep -qx "$((n + 1))+" <<<"$table_keys" || { echo "$doc (schema table): missing the row \"$((n + 1))+\" for future changes" >&2; fail=1; }

if [ "$fail" -ne 0 ]; then
    echo "Describe every schema step in the module comment of $schema and in the table of $doc." >&2
    exit 1
fi
