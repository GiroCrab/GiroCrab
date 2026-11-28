#!/usr/bin/env python3
"""Copies the browser scripts of the Advanzia connector (#935) that the Linux flow uses
into GiroCrabLinux/qml/AdvanziaWebLoginFlow.qml, between the "BEGIN generated" and
"END generated" markers, as JavaScript template literals:

- hook:             AdvanziaScripts.hook (user script, main frame, document start, page world)
- inspect:          AdvanziaAuthenticationScripts.inspect (structural login state only)
- discover, openTransactions, collect, statementScript, cancelScript: the scripts of AdvanziaBrowserSession

The Linux flow fills nothing into the bank's pages: the scripts that type the password
(AdvanziaLoginScripts.credentials), pick the release method (AdvanziaLoginScripts.method)
or submit the SMS code (AdvanziaAuthenticationScripts.submit) are deliberately not copied;
the user signs in and confirms in the browser window.

Run after changing the Swift scripts:  python3 -I scripts/gen-advanzia-qml.py
scripts/test-advanzia-linux.cjs (CI) fails while both differ.
"""
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
UI = ROOT / "GiroCrabApp/Sources/GiroCrabUI"
QML = ROOT / "GiroCrabLinux/qml/AdvanziaWebLoginFlow.qml"
NAMES = ["hook", "inspect", "discover", "openTransactions", "collect", "statementScript", "cancelScript"]
BEGIN, END = "    // BEGIN generated: Advanzia scripts", "    // END generated"


def dedent(body, indent):
    """Swift strips the closing delimiter's indentation from every line of a multi-line string."""
    pad = " " * indent
    return "\n".join(l[indent:] if l.startswith(pad) else l.lstrip(" ") for l in body.split("\n"))


def raw(file, name):
    """A `static let <name> = #\"\"\"…\"\"\"#` raw string (no escapes are processed in it)."""
    src = (UI / file).read_text()
    m = re.search(r'static let ' + name + r' = #"""\n([\s\S]*?)\n    """#', src)
    if not m:
        raise SystemExit(f"{file}: {name} not found")
    return dedent(m.group(1), 4)


def plain(file, pattern, what):
    """A one-line Swift string literal without escapes or interpolation."""
    m = re.search(pattern, (UI / file).read_text())
    if not m:
        raise SystemExit(f"{file}: {what} not found")
    if "\\" in m.group(1):
        raise SystemExit(f"{file}: {what} has escapes; extend the generator")
    return m.group(1)


def swift_scripts():
    session = "AdvanziaBrowserSession.swift"
    return {
        "hook": raw("AdvanziaScripts.swift", "hook"),
        "inspect": raw("AdvanziaAuthentication.swift", "inspect"),
        "discover": plain(session, r'let script = "([^"\n]*discover\(\)[^"\n]*)"', "discover"),
        "openTransactions": plain(session, r'callAsyncJavaScript\("([^"\n]*openTransactions\(\)[^"\n]*)"', "openTransactions"),
        "collect": plain(session, r'let script = "([^"\n]*\.collect\(account\);)"', "collect"),
        "statementScript": plain(session, r'static let statementScript = "([^"\n]*)"', "statement"),
        "cancelScript": plain(session, r'evaluateJavaScript\("([^"\n]*cancel\(\))"', "cancel"),
    }


def template_literal(s):
    return s.replace("\\", "\\\\").replace("`", "\\`").replace("${", "\\${")


def main():
    scripts = swift_scripts()
    block = "\n".join(f"    readonly property string {n}: `{template_literal(scripts[n])}`" for n in NAMES)
    text = QML.read_text()
    i, j = text.index(BEGIN), text.index(END)
    QML.write_text(text[:i] + BEGIN + " (AdvanziaScripts.swift, AdvanziaAuthentication.swift, AdvanziaBrowserSession.swift)\n"
                   + block + "\n" + text[j:])
    print(f"updated {QML.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
