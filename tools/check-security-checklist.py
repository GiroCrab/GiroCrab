#!/usr/bin/env python3
"""Fails a PR that touches security-critical paths without a ticked Security-Check (#1803).

Runs from the trusted base checkout (`.github/workflows/ai-transparency.yml`, job `security-checklist`);
the PR is read through the GitHub API as plain data and never executed.

  python3 tools/check-security-checklist.py --pr N [--repo OWNER/NAME]
"""
import argparse
import os
import re
import subprocess
import sys

# A path is critical when it equals an entry or starts with it (entries ending in `/` are directories,
# the others also cover `vault.rs` + `vault/`, `secret*.rs` and so on).
CRITICAL_PREFIXES = (
    "girocrab-core/src/vault",
    "girocrab-core/src/secret",
    "girocrab-core/src/lib.rs",  # generate_master_key(): the SQLCipher master key
    "girocrab-core/src/payment_input.rs",
    "girocrab-core/src/connectors/",
    "girocrab-core/src/local_api",
    "GiroCrabApp/Sources/GiroCrabUI/VaultKeyManager.swift",
    "GiroCrabApp/Sources/GiroCrabUI/BankingPolicy.swift",
    "GiroCrabApp/Sources/GiroCrabUI/AppModel+LocalApi.swift",
    "GiroCrabLinux/src/local_api.rs",
    "GiroCrabLinux/src/vault_key.rs",
    "GiroCrabWindows/src/vault_key.rs",
    "GiroCrabAndroid/app/src/main/java/de/girocrab/app/security/",
    ".github/workflows/",
    "scripts/",
)
HTML_COMMENT_RE = re.compile(r"<!--.*?-->", re.S)
# Fenced code blocks (``` or ~~~). As in CommonMark, the closing fence uses the opener's character, is at least
# as long (```` closes ```, ``` does not close ````) and is indented by at most three spaces; a backtick opener's
# info string holds no backtick (```x``` is inline code, not a fence); an unclosed fence runs to the end of the text.
# The opener is deliberately NOT limited to three spaces: fences inside list items are indented further, and their
# examples must not count as a ticked box. Over-stripping only fails the check (fail-safe), never lets it pass.
FENCE_RE = re.compile(
    r"^[ \t]*(?:(`{3,})(?=[^`\n]*$).*?(?:^ {0,3}\1`*[ \t]*$|\Z)|(~{3,}).*?(?:^ {0,3}\2~*[ \t]*$|\Z))", re.S | re.M
)
# `- [x] **Security-Check:** <what was checked>` (also `* [X] Security-Check: ok`); an unticked `[ ]`, a plain
# bullet or a ticked label without an explanation on the same line does not count.
CHECKED_RE = re.compile(r"^[ \t]*[-*][ \t]*\[[xX]\][ \t]*\**[ \t]*Security-Check\b[*: \t]*[^\s*:]", re.M)
LIST_ITEM_RE = re.compile(r"^ {0,3}(?:[-*+]|\d{1,9}[.)])(?:[ \t]|$)")


def strip_indented_code(text):
    """Drops lines indented by 4+ spaces or a tab unless they belong to a list item: outside a list they are an
    indented code block (or paragraph text), never a checkbox; inside a list they are nested items and stay."""
    out, in_list, prev_blank = [], False, True
    for line in text.split("\n"):
        if not line.strip():
            out.append(line)
            prev_blank = True
            continue
        if line.startswith(("    ", "\t")):
            if in_list:
                out.append(line)
            prev_blank = False
            continue
        if LIST_ITEM_RE.match(line):
            in_list = True
        elif prev_blank:
            in_list = False
        out.append(line)
        prev_blank = False
    return "\n".join(out)


def critical_paths(paths):
    return sorted({p for p in paths if p.startswith(CRITICAL_PREFIXES)})


def has_checked_security_check(body):
    """HTML comments are template hints and fenced or indented code blocks are examples, so a ticked box inside
    any of them does not count (a nested item under a list item does); the ticked box needs an explanation after
    the label."""
    text = strip_indented_code(FENCE_RE.sub("", HTML_COMMENT_RE.sub("", body or "")))
    return bool(CHECKED_RE.search(text))


def verdict(paths, body):
    """-> (ok, message)"""
    hits = critical_paths(paths)
    if not hits:
        return True, "No security-critical paths changed; no Security-Check required."
    if has_checked_security_check(body):
        return True, f"{len(hits)} security-critical file(s) changed; Security-Check is ticked."
    shown = "\n".join(f"  - {p}" for p in hits[:20]) + ("\n  - …" if len(hits) > 20 else "")
    return False, (
        "This PR changes security-critical files but its description has no ticked and explained Security-Check.\n"
        f"{shown}\n"
        "Add this line to the PR description (under '### 🤖 AI Process Reflection' or on its own) and say what "
        "you checked (secrets/personal data, new network/process/file access, dependencies), "
        "\"Nothing relevant: <why>\" is valid:\n"
        "  - [x] **Security-Check:** <what was checked>"
    )


def gh(*args):
    r = subprocess.run(["gh", "api", *args], capture_output=True, text=True)
    if r.returncode:
        sys.exit(f"gh api failed: {r.stderr.strip()}")
    return r.stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pr", required=True)
    ap.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "GiroCrab/GiroCrab"))
    a = ap.parse_args()
    files = gh(f"repos/{a.repo}/pulls/{a.pr}/files", "--paginate", "--jq",
               ".[] | .filename, (.previous_filename // empty)").split("\n")
    body = gh(f"repos/{a.repo}/pulls/{a.pr}", "--jq", ".body // \"\"")
    ok, msg = verdict([f for f in files if f], body)
    if ok:
        print(msg)
        return 0
    print(f"::error title=Security-Check missing::{msg.splitlines()[0]}")
    print(msg)
    return 1


if __name__ == "__main__":
    sys.exit(main())
