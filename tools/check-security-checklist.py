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
    "girocrab-core/src/sepa.rs",
    "girocrab-core/src/payment_input.rs",
    "girocrab-core/src/connectors/",
    "girocrab-core/src/local_api/",
    "GiroCrabApp/Sources/GiroCrabUI/VaultKeyManager.swift",
    "GiroCrabApp/Sources/GiroCrabUI/BankingPolicy.swift",
    ".github/workflows/",
    "scripts/",
)
HTML_COMMENT_RE = re.compile(r"<!--.*?-->", re.S)
# `- [x] **Security-Check:** …` (also `* [X] Security-Check`); an unticked `[ ]` or a plain bullet does not count.
CHECKED_RE = re.compile(r"^\s*[-*]\s*\[[xX]\]\s*\**\s*Security-Check\b", re.M)


def critical_paths(paths):
    return sorted({p for p in paths if p.startswith(CRITICAL_PREFIXES)})


def has_checked_security_check(body):
    """HTML comments are template hints, so a ticked box inside one does not count."""
    return bool(CHECKED_RE.search(HTML_COMMENT_RE.sub("", body or "")))


def verdict(paths, body):
    """-> (ok, message)"""
    hits = critical_paths(paths)
    if not hits:
        return True, "No security-critical paths changed; no Security-Check required."
    if has_checked_security_check(body):
        return True, f"{len(hits)} security-critical file(s) changed; Security-Check is ticked."
    shown = "\n".join(f"  - {p}" for p in hits[:20]) + ("\n  - …" if len(hits) > 20 else "")
    return False, (
        "This PR changes security-critical files but its description has no ticked Security-Check.\n"
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
