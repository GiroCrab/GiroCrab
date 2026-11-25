---
name: girocrab-preflight
description: Pre-commit and pre-PR verification gate for GiroCrab. Run automatically before committing, pushing, or opening/updating a pull request to prevent CI failures, missing translations, privacy leaks, and AI attribution card errors.
---

# GiroCrab Preflight Gate

Run only the checks matching your touched paths before committing or pushing:

1. **Diff Discipline:**
   - Check `git diff --stat`: keep focused PRs under ~300 lines and ~15 files (`size L` triggers automatic review holds).
   - Confirm no unrelated files, generated bindings, or unrequested refactors slipped in.

2. **Fast Repo Lints (always run, or via `bash scripts/verify-fast.sh --lint-only`):**
   - `cargo fmt --all -- --check`
   - `bash scripts/lint-brand-names.sh`
   - `bash scripts/lint-connectors.sh`
   - `bash scripts/lint-no-obfuscation.sh`
   - `bash scripts/check-third-party-notices.sh`

3. **Path-Specific Checks:**
   - **`girocrab-core/**`:**
     - `bash scripts/lint-serde-feature.sh`
     - `GIROCRAB_SKIP_SWIFT=1 cargo check -q -p girocrab-core --features serde`
     - `cargo clippy -q -p girocrab-core --all-targets -- -D warnings`
     - `cargo test -q -p girocrab-core --lib <touched_module>`
     - `python3 -I GiroCrabLinux/scripts/i18n-report.py`
   - **`GiroCrabApp/**`:**
     - `bash scripts/lint-vault-access.sh && bash scripts/lint-fonts.sh && bash scripts/lint-report-dialogs.sh`
     - If `Localizable.xcstrings` changed: `python3 scripts/xcstrings.py check`
     - `cd GiroCrabApp && swift test -q --filter <TouchedSuite>` (mark `UserDefaults.standard` tests with `// test: serial-defaults`)
   - **`GiroCrabLinux/**`:**
     - `python3 -I GiroCrabLinux/scripts/i18n-report.py` (must report 0 missing English strings in `en-linux.json`)
     - `cargo fmt --manifest-path GiroCrabLinux/Cargo.toml -- --check`
   - **`GiroCrabWindows/**`:**
     - Ensure no `#![cfg(windows)]`-only API lacks a cross-platform stub so macOS/Linux `clippy` & `test` build cleanly.
     - `cargo fmt --all --manifest-path GiroCrabWindows/Cargo.toml -- --check`
     - `cargo clippy -q --manifest-path GiroCrabWindows/Cargo.toml --all-targets -- -D warnings`
     - `cargo test -q --manifest-path GiroCrabWindows/Cargo.toml`

4. **Privacy & Redaction Invariant:**
   - Every new UI surface showing IBANs, account/owner names, balances, amounts, or IBAN-derived bank names MUST be masked in privacy/report mode AND block clipboard/file export while privacy mode or vault lock is active.
   - Multi-currency totals never sum across currencies (`EUR` primary, foreign currencies listed separately).

5. **AI Attribution & German PR Conventions:**
   - Commit trailers include `AI-Orchestrator:`, `AI-Agent:` / `AI-Model:`, and matching `Co-authored-by:`.
   - PR title (conventional commit prefix) and body in German, ending with `### 🤖 AI Process Reflection` (`Human-Steering`, `AI-Agency`, `Review-Focus`, `Security-Check`, `Perf-Check`).
