#!/usr/bin/env bash
# The schema steps in girocrab-core/src/schema.rs and their descriptions stay in sync:
# every step 1..N of `STEPS` has a line in the module comment of schema.rs and a row in
# the table of docs/ARCHITECTURE.md, and the table's "future changes" row is "N+1+".
set -euo pipefail
cd "$(dirname "$0")/.."
schema=girocrab-core/src/schema.rs
doc=docs/ARCHITECTURE.md

# Counts every `Step::…` between `const STEPS` and its closing `];` (read from stdin), also
# several on one line. A character scanner keeps its state across lines: string literals
# (with escapes, also over several lines), `//` comments and `/* … */` comments (nested, as
# in Rust) are left out, so a `//` in a URL or a commented-out step does not change the
# count. Raw strings (`r"…"`, `r#"…"#`) are not parsed: the scan fails instead of miscounting.
count_steps() {
    awk '
    /^const STEPS: / { on = 1; next }
    on && depth == 0 && !str && /^\];/ { exit }
    on {
        code = ""
        len = length($0)
        for (i = 1; i <= len; i++) {
            ch = substr($0, i, 1); two = substr($0, i, 2)
            if (str) {
                if (ch == "\\") i++
                else if (ch == "\"") str = 0
            } else if (depth > 0) {
                if (two == "/*") { depth++; i++ }
                else if (two == "*/") { depth--; i++ }
            } else if (two == "//") {
                break
            } else if (two == "/*") {
                depth = 1; i++
            } else if (ch == "\"") {
                prev = substr($0, i - 1, 1)
                if (prev == "r" || prev == "#") {
                    print "line " NR ": raw string literal in STEPS is not supported (update scripts/lint-schema-docs.sh)" > "/dev/stderr"
                    bad = 1; exit
                }
                str = 1
            } else code = code ch
        }
        c += gsub(/Step::/, "", code)
    }
    END { if (bad) exit 1; print c + 0 }'
}

# Self-test of the scanner on fixed samples (milliseconds; runs on every lint).
expect_count() { # expected count, body of STEPS
    local got
    got=$(printf 'const STEPS: &[Step] = &[\n%s\n];\n' "$2" | count_steps)
    [ "$got" = "$1" ] || {
        echo "$0 self-test: expected $1 steps, counted $got in:" >&2
        printf '%s\n' "$2" >&2
        exit 1
    }
}
expect_count 2 $'    Step::Sql(A), Step::Sql(B), // 1, 2'
expect_count 1 $'    Step::Sql(A), // Step::Sql(Z)'
expect_count 1 $'    /* Step::Hook(x), */ Step::Sql(A),'
expect_count 1 $'    /* old /* Step::Sql(B) */\n    Step::Sql(C), */\n    Step::Sql(A),'
expect_count 2 $'    Step::Sql("http://x"), Step::Sql(B),'
expect_count 2 $'    Step::Sql("a\\"b // c"), Step::Sql(B),'
expect_count 2 $'    Step::Sql("a\n    Step::Sql(X) // y"), Step::Sql(B),'
expect_count 1 $'    Step::Sql("];"),\n];\n    Step::Sql(B),'
if printf 'const STEPS: &[Step] = &[\n    Step::Sql(r"x"),\n];\n' | count_steps >/dev/null 2>&1; then
    echo "$0 self-test: a raw string in STEPS must fail the scan" >&2
    exit 1
fi

n=$(count_steps <"$schema")
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

# Only the table in the section "## Schema & migrations", not other tables of the file.
table_keys=$(awk '/^## /{on = ($0 == "## Schema & migrations")} on' "$doc" |
    sed -n 's/^| *\([0-9][0-9–+-]*\) *|.*/\1/p' | expand)
check "$doc (schema table)" "$table_keys" || fail=1
grep -qx "$((n + 1))+" <<<"$table_keys" || { echo "$doc (schema table): missing the row \"$((n + 1))+\" for future changes" >&2; fail=1; }

if [ "$fail" -ne 0 ]; then
    echo "Describe every schema step in the module comment of $schema and in the table of $doc." >&2
    exit 1
fi
echo "lint-schema-docs: ok"
