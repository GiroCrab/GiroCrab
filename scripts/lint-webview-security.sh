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
#    Hosts are deliberately not checked here: consent pages belong to whichever bank the
#    user picked and portal logins may hand over to bank SSO domains. Host binding is
#    enforced on the data paths (check 4), not on main-frame navigation.
for f in $(grep -lE 'WKWebView\(' "$SWIFT_DIR"/*.swift); do
    is_local "$f" && continue
    grep -q 'decidePolicyFor' "$f" || err "$f: WKWebView without decidePolicyFor navigation guard"
    grep -qE 'allowsMainFrame|== *"https"' "$f" || err "$f: navigation guard does not restrict the scheme to https"
done

# 2. QML: a WebEngineView must reject navigations and restrict the scheme to https (a bare
#    handler is not enough); the guard must not be switched off.
QML_SCHEME_RE='startsWith\(.https:.\)|protocol *[!=]==? *.https:.'
for bad in 'onNavigationRequested: request => { }'; do
    ! grep -qE "$QML_SCHEME_RE" <<<"$bad" || err "self-test: handler without scheme check accepted: $bad"
done
for ok in 'if (request.isMainFrame && !u.toLowerCase().startsWith("https:")) request.reject()' \
          'if (a.protocol !== "https:") request.reject()'; do
    grep -qE "$QML_SCHEME_RE" <<<"$ok" || err "self-test: https scheme check not recognised: $ok"
done
QML_VIEW_RE='^[[:space:]]*(sourceComponent: )?WebEngineView[[:space:]]*\{|^WebEngineView[[:space:]]*\{'
# every WebEngineView needs its own handler: as many `onNavigationRequested` as views in the file
qml_guard_gap() {
    local v h
    v=$(grep -cE "$QML_VIEW_RE" <<<"$1" || true)
    h=$(grep -c 'onNavigationRequested' <<<"$1" || true)
    (( h < v ))
}
qml_guard_gap $'WebEngineView {\n  onNavigationRequested: r => { if (!r.url.toString().startsWith("https:")) r.reject() }\n}\nWebEngineView {\n}' \
    || err "self-test: second WebEngineView without handler accepted"
if qml_guard_gap $'WebEngineView {\n  onNavigationRequested: r => { }\n}\nWebEngineView {\n  onNavigationRequested: r => { }\n}'; then
    err "self-test: one handler per WebEngineView flagged"
fi
for f in $(grep -lE "$QML_VIEW_RE" "$QML_DIR"/*.qml); do
    is_local "$f" && continue
    grep -q 'onNavigationRequested' "$f" || err "$f: WebEngineView without onNavigationRequested guard"
    ! qml_guard_gap "$(cat "$f")" || err "$f: WebEngineView without its own onNavigationRequested guard"
    grep -qE "$QML_SCHEME_RE" "$f" || err "$f: WebEngineView navigation guard does not restrict the scheme to https"
done
hits=$(grep -nE 'navigationGuard: *false' "$QML_DIR"/*.qml || true)
[[ -z "$hits" ]] || err "navigation guard switched off:"$'\n'"$hits"

# 3. Scripts that look at a password field: origin/host gate in the same file, and no read of its value
#    (neither through a variable `pw`/`password…` nor straight from the selector,
#    `querySelector('input[type=password]').value`). Writes stay allowed.
PW_READ_RE='\b(pw|pwd|pass(word)?[A-Za-z]*)\.value\b|type[[:space:]]*=[^]]*password[^)]*\)\.value\b'
# every `.value` access is classified on its own: one followed by an assignment (or `).set`) is a
# permitted write, any other one a read – so `old = pw.value; pw.value = next;` is still flagged.
# The assignment is matched as `=` / `==` without consuming the next character, so the start of a
# chained read (`pw.value=pwd.value`) stays visible; `==` is a comparison and therefore a read.
PW_OP_RE="($PW_READ_RE)( *==?|\)\.set)?"
PW_WRITE_END='value( *=|\)\.set)$'
pw_reads() {
    local line
    while IFS= read -r line; do
        [[ -n "$(grep -oE "$PW_OP_RE" <<<"$line" | grep -vE "$PW_WRITE_END" || true)" ]] && printf '%s\n' "$line"
    done
    return 0
}
# password fields a script addresses (any quoting / spacing of the attribute selector)
PW_FIELD_RE='type[[:space:]]*={1,3}[[:space:]]*(\\)?.?password'
# self-test (fixtures): every read form must be flagged, permitted writes must not
for bad in "var v = pw.value;" \
           "var v = document.querySelector('input[type=password]').value;" \
           "var v = document.querySelector('input[type=\"password\"]').value;" \
           "var v = document.querySelector('input[type = password]').value;" \
           "const old = pw.value; pw.value = next;" \
           "pw.value=pwd.value;" \
           "if (pw.value==x) { go(); }"; do
    [[ -n "$(pw_reads <<<"$bad")" ]] || err "self-test: password read not detected: $bad"
done
for ok in "pw.value = v;" \
          "pw.value=v;" \
          "document.querySelector('input[type=password]').value = v;" \
          "document.querySelector(\"input[type='password']\").value = v;"; do
    [[ -z "$(pw_reads <<<"$ok")" ]] || err "self-test: password write flagged: $ok"
done
# the origin-gate discovery must see every spelling of the selector
for field in "input[type=password]" \
             "input[type=\"password\"]" \
             "document.querySelector(\"input[type='password']\").value = secret;" \
             "input[type = 'password']" \
             "el.type === 'password'" \
             "input[type=\\\"password\\\"]"; do
    grep -qE "$PW_FIELD_RE" <<<"$field" || err "self-test: password field not discovered: $field"
done
for f in $(grep -lE "$PW_FIELD_RE" "$SWIFT_DIR"/*.swift "$QML_DIR"/*.qml 2>/dev/null); do
    grep -qE 'location\.(origin|hostname)' "$f" || err "$f: password field script without location.origin/hostname gate"
done
hits=$(grep -nE "$PW_READ_RE" "$SWIFT_DIR"/*.swift "$QML_DIR"/*.qml | pw_reads || true)
[[ -z "$hits" ]] || err "script reads a password field value:"$'\n'"$hits"

# 4. Data paths must stay bound to the portal's domains.
grep -q 'domains.allows(m.frameInfo.securityOrigin.host)' "$SWIFT_DIR/PortalUI.swift" || err "PortalUI.swift: message handler lost its domain check"
grep -q 'AppModel.domainAllows(flow.domains' "$QML_DIR/WebLoginSheet.qml" || err "WebLoginSheet.qml: onPortal lost its domain check"

[[ $fail -eq 0 ]] || exit 1
echo "lint-webview-security: ok"
