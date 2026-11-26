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

# 1. Swift: a file creating a remote WKWebView must decide navigations (https only), and every
#    `decidePolicyFor navigationAction` must use the scheme guard itself: call
#    `ConsentRedirect.allowsMainFrame` (or compare with "https") and cancel. An unused helper
#    next to an unconditional `.allow` does not count.
#    Hosts are deliberately not checked here: consent pages belong to whichever bank the
#    user picked and portal logins may hand over to bank SSO domains. Host binding is
#    enforced on the data paths (check 4), not on main-frame navigation.
SWIFT_SCHEME_RE='allowsMainFrame\(|== *"https"'
swift_policies() {   # stdin = Swift source; each `decidePolicyFor navigationAction` body on one line
    awk '!on && /decidePolicyFor navigationAction/ { on = 1; d = 0; s = 0; b = "" }
         on { b = b " " $0; o = gsub(/\{/, "{"); c = gsub(/\}/, "}"); d += o - c; if (o) s = 1
              if (s && d <= 0) { print b; on = 0 } }'
}
swift_guard_gap() {   # $1 = Swift source; succeeds when a navigation policy is missing or skips the https guard
    local body found=0
    while IFS= read -r body; do
        found=1
        grep -qE "$SWIFT_SCHEME_RE" <<<"$body" && grep -q '\.cancel' <<<"$body" || return 0
    done < <(swift_policies <<<"$1")
    (( found == 0 ))
}
swift_guard_gap $'static func allowsMainFrame(_ url: URL?) -> Bool { url?.scheme?.lowercased() == "https" }\nfunc webView(_ w: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,\n             decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {\n    decisionHandler(.allow)\n}' \
    || err "self-test: unused allowsMainFrame helper with an unconditional .allow accepted"
if swift_guard_gap $'func webView(_ w: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,\n             decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {\n    let ok = ConsentRedirect.allowsMainFrame(navigationAction.request.url)\n    decisionHandler(ok ? .allow : .cancel)\n}'; then
    err "self-test: navigation policy using allowsMainFrame flagged"
fi
for f in $(grep -lE 'WKWebView\(' "$SWIFT_DIR"/*.swift); do
    is_local "$f" && continue
    ! swift_guard_gap "$(cat "$f")" \
        || err "$f: WKWebView without a decidePolicyFor guard that cancels non-https main-frame navigations (allowsMainFrame)"
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
QML_VIEW_RE='^[[:space:]]*(sourceComponent:[[:space:]]*)?WebEngineView[[:space:]]*\{|^WebEngineView[[:space:]]*\{'
# every WebEngineView needs its own handler, and that handler must check the scheme itself or call a
# function of the file that does (`browser.navigation(…)` in ConsentBrowser.qml)
qml_views() {   # stdin = QML; each WebEngineView block on one line
    QML_VIEW_RE="$QML_VIEW_RE" awk '
        !on && $0 ~ ENVIRON["QML_VIEW_RE"] { on = 1; d = 0; b = "" }
        on { b = b " " $0; d += gsub(/\{/, "{") - gsub(/\}/, "}"); if (d <= 0) { print b; on = 0 } }'
}
qml_function() {   # stdin = QML, $1 = name; the body of `function name(` on one line
    awk -v n="$1" '
        !on && index($0, "function " n "(") { on = 1; d = 0; s = 0; b = "" }
        on { b = b " " $0; o = gsub(/\{/, "{"); c = gsub(/\}/, "}"); d += o - c; if (o) s = 1
             if (s && d <= 0) { print b; on = 0 } }'
}
qml_guard_gap() {   # $1 = QML; succeeds when a WebEngineView lacks its own https navigation guard
    local view handler name
    while IFS= read -r view; do
        [[ "$view" == *onNavigationRequested* ]] || return 0
        handler=$(sed -E 's/.*onNavigationRequested://; s/[[:space:]]on[A-Z][A-Za-z]*:.*//' <<<"$view")
        grep -qE "$QML_SCHEME_RE" <<<"$handler" && continue
        for name in $(grep -oE '[A-Za-z_][A-Za-z0-9_]*\(' <<<"$handler" | tr -d '('); do
            grep -qE "$QML_SCHEME_RE" <<<"$(qml_function "$name" <<<"$1")" && continue 2
        done
        return 0
    done < <(qml_views <<<"$1")
    return 1
}
qml_guard_gap $'WebEngineView {\n  onNavigationRequested: r => { if (!r.url.toString().startsWith("https:")) r.reject() }\n}\nWebEngineView {\n}' \
    || err "self-test: second WebEngineView without handler accepted"
qml_guard_gap $'WebEngineView {\n  onNavigationRequested: r => { if (!r.url.toString().startsWith("https:")) r.reject() }\n}\nWebEngineView {\n  onNavigationRequested: r => { }\n}' \
    || err "self-test: second WebEngineView with a handler without scheme check accepted"
if qml_guard_gap $'Item {\n  WebEngineView {\n    onNavigationRequested: request => nav(request)\n  }\n  WebEngineView {\n    onNavigationRequested: request => nav(request)\n  }\n  function nav(request) {\n    if (request.isMainFrame && !request.url.toString().startsWith("https:")) request.reject()\n  }\n}'; then
    err "self-test: WebEngineViews sharing a guard function flagged"
fi
grep -qE "$QML_VIEW_RE" <<<'    sourceComponent:  WebEngineView {' \
    || err "self-test: WebEngineView after sourceComponent with extra whitespace not discovered"
qml_guard_gap $'Item {\n  sourceComponent:  WebEngineView {\n  }\n}' \
    || err "self-test: sourceComponent WebEngineView with extra whitespace and no handler accepted"
for f in $(grep -lE "$QML_VIEW_RE" "$QML_DIR"/*.qml); do
    is_local "$f" && continue
    ! qml_guard_gap "$(cat "$f")" \
        || err "$f: WebEngineView without its own onNavigationRequested guard restricting the scheme to https"
done
hits=$(grep -nE 'navigationGuard: *false' "$QML_DIR"/*.qml || true)
[[ -z "$hits" ]] || err "navigation guard switched off:"$'\n'"$hits"

# 3. Scripts that look at a password field: each script carries its own origin/host gate, and none reads
#    the field's value (neither through a variable `pw`/`password…`, nor straight from the selector,
#    `querySelector('input[type=password]').value`, nor through a name bound to such a selector,
#    `const field = querySelector('input[type=password]'); field.value`). Writes stay allowed.
PW_READ_RE='\b(pw|pwd|pass(word)?[A-Za-z]*)\.value\b|type[[:space:]]*=[^]]*password[^)]*\)\.value\b'
# password fields a script addresses (any quoting / spacing of the attribute selector)
PW_FIELD_RE='type[[:space:]]*={1,3}[[:space:]]*(\\)?.?password'
# names bound to a password field: their `.value` is the password, too
PW_BIND_RE="\b(var|let|const)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=[^;]*($PW_FIELD_RE)"
# every `.value` access is classified on its own: one followed by an assignment (or `).set`) is a
# permitted write, any other one a read – so `old = pw.value; pw.value = next;` is still flagged.
# The assignment is matched as `=` / `==` without consuming the next character, so the start of a
# chained read (`pw.value=pwd.value`) stays visible; `==` is a comparison and therefore a read.
PW_WRITE_END='value( *=|\)\.set)$'
pw_aliases() {   # stdin = script; the names bound to a password field as `a|b`
    { grep -oE "$PW_BIND_RE" || true; } \
        | sed -E 's/^(var|let|const)[[:space:]]+([A-Za-z_][A-Za-z0-9_]*).*/\2/' | sort -u | paste -s -d '|' -
}
pw_reads() {   # stdin = lines, $1 = bound names (`a|b`, optional); prints the lines that read a value
    local line re="$PW_READ_RE"
    [[ -z "${1:-}" ]] || re="\b($1)\.value\b|$re"
    while IFS= read -r line; do
        [[ -n "$(grep -oE "($re)( *==?|\)\.set)?" <<<"$line" | grep -vE "$PW_WRITE_END" || true)" ]] && printf '%s\n' "$line"
    done
    return 0
}
pw_scan() {   # stdin = script or file; prints the numbered lines that read a password field value
    local text names re
    text=$(cat)
    names=$(pw_aliases <<<"$text")
    re=$PW_READ_RE; [[ -z "$names" ]] || re="\b($names)\.value\b|$re"
    { grep -nE "$re" <<<"$text" || true; } | pw_reads "$names"
}
# self-test (fixtures): every read form must be flagged, permitted writes must not
for bad in "var v = pw.value;" \
           "var v = document.querySelector('input[type=password]').value;" \
           "var v = document.querySelector('input[type=\"password\"]').value;" \
           "var v = document.querySelector('input[type = password]').value;" \
           "const old = pw.value; pw.value = next;" \
           "pw.value=pwd.value;" \
           "if (pw.value==x) { go(); }" \
           $'const field = document.querySelector(\'input[type=password]\');\nconst value = field.value;'; do
    [[ -n "$(pw_scan <<<"$bad")" ]] || err "self-test: password read not detected: $bad"
done
for ok in "pw.value = v;" \
          "pw.value=v;" \
          "document.querySelector('input[type=password]').value = v;" \
          "document.querySelector(\"input[type='password']\").value = v;" \
          $'const field = document.querySelector(\'input[type=password]\');\nfield.value = secret;'; do
    [[ -z "$(pw_scan <<<"$ok")" ]] || err "self-test: password write flagged: $ok"
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
# the gate is checked per script: each multi-line string literal (""" in Swift, ` in QML) that
# addresses a password field needs its own location.origin/hostname check
PW_GATE_RE='location\.(origin|hostname)'
pw_segments() {   # stdin = file, $1 = literal delimiter; each literal on one line, last the code outside them
    awk -v d="$1" '{ n = gsub(d, d) }
        inl { lit = lit " " $0 }
        !inl { out = out " " $0 }
        n % 2 { if (inl) { print lit; lit = "" } inl = !inl }
        END { if (inl) print lit; print out }'
}
pw_gate_gap() {   # $1 = file, $2 = literal delimiter; succeeds when a script touches a password field ungated
    local seg segs
    segs=$(pw_segments "$2" <<<"$1")
    while IFS= read -r seg; do
        grep -qE "$PW_FIELD_RE" <<<"$seg" && ! grep -qE "$PW_GATE_RE" <<<"$seg" && return 0
    done < <(sed '$d' <<<"$segs")
    # code outside multi-line literals (one-line scripts): every line that addresses a password field
    # carries the gate itself (a gate elsewhere in the file does not cover it)
    local out line
    out=$(pw_outside "$2" <<<"$1")
    while IFS= read -r line; do
        grep -qE "$PW_FIELD_RE" <<<"$line" && ! grep -qE "$PW_GATE_RE" <<<"$line" && return 0
    done <<<"$out"
    return 1
}
pw_outside() {   # stdin = file, $1 = literal delimiter; the lines outside multi-line literals
    awk -v d="$1" '{ n = gsub(d, d) } !inl { print } n % 2 { inl = !inl }'
}
gate_ok=$'let a = """\nif (location.hostname !== host) return false;\ndocument.querySelector(\'input[type=password]\').value = pw;\n"""'
pw_gate_gap "$gate_ok"$'\nlet b = """\ndocument.querySelector(\'input[type=password]\').value = pw;\n"""' '"""' \
    || err "self-test: second password script without its own origin gate accepted"
if pw_gate_gap "$gate_ok" '"""'; then
    err "self-test: origin-gated password script flagged"
fi
pw_gate_gap $'let a = "if (location.hostname == h) {}"\nlet b = "document.querySelector(\'input[type=password]\').value = pw;"' '"""' \
    || err "self-test: one-line password script gated only by another line accepted"
if pw_gate_gap $'let a = "location.hostname == h && document.querySelector(\'input[type=password]\').value = pw;"' '"""'; then
    err "self-test: gated one-line password script flagged"
fi
for f in $(grep -lE "$PW_FIELD_RE" "$SWIFT_DIR"/*.swift "$QML_DIR"/*.qml 2>/dev/null); do
    d='"""'; [[ "$f" == *.qml ]] && d='`'
    ! pw_gate_gap "$(cat "$f")" "$d" || err "$f: password field script without its own location.origin/hostname gate"
done
hits=""
for f in $(grep -lE "$PW_READ_RE|$PW_FIELD_RE" "$SWIFT_DIR"/*.swift "$QML_DIR"/*.qml 2>/dev/null); do
    h=$(pw_scan <"$f")
    [[ -z "$h" ]] || hits+=$(sed "s|^|$f:|" <<<"$h")$'\n'
done
[[ -z "$hits" ]] || err "script reads a password field value:"$'\n'"$hits"

# 4. Data paths must stay bound to the portal's domains.
grep -q 'domains.allows(m.frameInfo.securityOrigin.host)' "$SWIFT_DIR/PortalUI.swift" || err "PortalUI.swift: message handler lost its domain check"
grep -q 'AppModel.domainAllows(flow.domains' "$QML_DIR/WebLoginSheet.qml" || err "WebLoginSheet.qml: onPortal lost its domain check"

[[ $fail -eq 0 ]] || exit 1
echo "lint-webview-security: ok"
