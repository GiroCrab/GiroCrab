---
name: girocrab-ui-parity
description: Guidelines for porting features from the macOS reference app (GiroCrabApp) to GiroCrabLinux (Qt/QML), GiroCrabWindows (Slint), iOS, or GiroCrabAndroid (Jetpack Compose). Use whenever implementing cross-platform UI parity.
---

# GiroCrab Cross-Platform UI Parity

1. **Read the Reference & Core First:**
   - Inspect the macOS reference implementation in `GiroCrabApp/Sources/GiroCrabUI/` and the exact `Vault` / model API in `girocrab-core/src/`.
   - Never duplicate business logic, parsers, or SQL queries in a client UI if `girocrab-core` already provides them.

2. **Non-Blocking Vault Access:**
   - Never run slow or mutating `Vault` operations synchronously on the main/UI thread. Dispatch through the client's worker/background helper (`vaultOffMain` on Apple, worker channel on Windows/Linux, `Dispatchers.IO` on Android).

3. **Platform-Specific Guardrails:**
   - **Windows (`GiroCrabWindows`):** Respect existing Slint component styles (`GhostButton` vs `Button`) and keep dialog rows within `780px` width. Provide non-Windows fallbacks for any Win32 integration so `cargo test` runs on macOS/Linux.
   - **Linux (`GiroCrabLinux`):** Add English translations to `en-linux.json` for every new user-facing string and verify keyboard navigation / focus frames on custom QML controls.
   - **macOS / iOS (`GiroCrabApp`):** Use `.appFont(...)` (never `.font(.caption)`), `.capturableSheet` / `.capturablePopover`, and `.sensitive(...)` modifiers.

4. **Privacy Mode & Tests:**
   - Mirror macOS privacy redaction (`PrivacyRedaction.swift`) in both display strings and copy/export actions.
   - Add 2–4 targeted unit tests in the client test suite verifying normal state, edge cases, and privacy redaction.
