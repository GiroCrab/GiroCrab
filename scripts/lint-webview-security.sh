#!/usr/bin/env bash
# Portal / consent WebViews (GitHub #1797): every web view that loads a remote page
# keeps a main-frame scheme guard (https only), and every injected script that
# touches a password field is bound to the portal's own origin and never reads it.
# Data paths stay host-bound: `PortalDomains` (Swift), `AppModel.domainAllows` (QML).
# Local-only web views (print / export of our own HTML) are listed in LOCAL_ONLY.
#
#   scripts/lint-webview-security.sh      exits 1 and lists offending files/lines
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SWIFT_DIR=GiroCrabApp/Sources/GiroCrabUI
QML_DIR=GiroCrabLinux/qml
LOCAL_ONLY=("$SWIFT_DIR/BookingExport.swift" "$SWIFT_DIR/PrintSheet.swift" "$QML_DIR/WebPrinter.qml")
fail=0
err() { echo "lint-webview-security: $*" >&2; fail=1; }
is_local() { local f; for f in "${LOCAL_ONLY[@]}"; do [[ "$f" == "$1" ]] && return 0; done; return 1; }

# 1. Swift: a file creating a remote WKWebView must decide navigations (https only).
for f in $(grep -lE 'WKWebView\(' "$SWIFT_DIR"/*.swift); do
    is_local "$f" && continue
    grep -q 'decidePolicyFor' "$f" || err "$f: WKWebView without decidePolicyFor navigation guard"
    grep -qE 'allowsMainFrame|== *"https"' "$f" || err "$f: navigation guard does not restrict the scheme to https"
done

# 2. QML: a WebEngineView must reject navigations; the guard must not be switched off.
for f in $(grep -lE '^[[:space:]]*(sourceComponent: )?WebEngineView[[:space:]]*\{|^WebEngineView[[:space:]]*\{' "$QML_DIR"/*.qml); do
    is_local "$f" && continue
    grep -q 'onNavigationRequested' "$f" || err "$f: WebEngineView without onNavigationRequested guard"
done
hits=$(grep -nE 'navigationGuard: *false' "$QML_DIR"/*.qml || true)
[[ -z "$hits" ]] || err "navigation guard switched off:"$'\n'"$hits"

# 3. Scripts that look at a password field: origin/host gate in the same file, and no read of its value.
for f in $(grep -lE "type *=== *'password'|input\[type=password\]|type=\"password\"" "$SWIFT_DIR"/*.swift "$QML_DIR"/*.qml 2>/dev/null); do
    grep -qE 'location\.(origin|hostname)' "$f" || err "$f: password field script without location.origin/hostname gate"
done
hits=$(grep -nE '\b(pw|pwd|pass(word)?[A-Za-z]*)\.value\b' "$SWIFT_DIR"/*.swift "$QML_DIR"/*.qml | grep -vE '\.value *=[^=]' | grep -vE '\.value\)\.set' || true)
[[ -z "$hits" ]] || err "script reads a password field value:"$'\n'"$hits"

# 4. Data paths must stay bound to the portal's domains.
grep -q 'domains.allows(m.frameInfo.securityOrigin.host)' "$SWIFT_DIR/PortalUI.swift" || err "PortalUI.swift: message handler lost its domain check"
grep -q 'AppModel.domainAllows(flow.domains' "$QML_DIR/WebLoginSheet.qml" || err "WebLoginSheet.qml: onPortal lost its domain check"

[[ $fail -eq 0 ]] || exit 1
echo "lint-webview-security: ok"
