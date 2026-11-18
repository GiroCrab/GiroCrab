#!/usr/bin/env python3
"""Wertet einen Ordner mit Bank- und Depot-PDFs lokal aus und schreibt einen geschwärzten Bericht.

Zweck: GiroCrab soll Dokumente der eigenen Banken (comdirect, DKB, Quirion, finvesto/FNZ, Trade Republic …)
besser typisieren und Buchungen zuordnen. Dafür braucht es nur die Struktur der Dokumente, nicht deren Inhalt.

Das Skript
  * liest die PDFs nur (nichts im Ordner wird verändert, nichts gelöscht oder verschoben),
  * benutzt kein Netzwerk,
  * schreibt EINEN Bericht (Markdown) – standardmäßig neben dem Skript,
  * schwärzt IBAN, Namen, Adressen, Mail, Telefon und lange Zahlen ab 7 Ziffern (Konto-/Depot-/Kundennummern),
  * lässt Beträge, Daten, ISIN und Wertpapiernamen stehen: sie sind der Kern der Zuordnung,
  * bricht ab und schreibt NICHTS, wenn der fertige Bericht noch eine IBAN, eine Mailadresse oder einen
    der angegebenen Namen enthält.

Aufruf:
  python3 scripts/analyse-dokumente.py "/Pfad/zum/Post-Ordner" --name "Vorname Nachname" --name "Partnername"
Optionen:
  --out bericht.md      Zieldatei (Standard: bericht.md neben dem Skript)
  --per-type 3          Beispiele je Bank und Dokumentart (Standard 3)
  --lines 18            Textzeilen je Beispiel (Standard 18)
  --bank NAME           nur diese Bank(en) auswerten (z. B. comdirect), mehrfach möglich
  --regeln datei.json   Regeldatei (Standard: girocrab-core/data/document_rules.json)
  --extractor BEFEHL    Text so lesen, wie die App ihn liest, z. B.
                        'cargo run -q -p girocrab-core --example pdf_text --' (Standard: pdftotext)
  --etikett TEXT        zählt je Art, in wie vielen Dokumenten das Etikett vorkommt (mehrfach möglich)
  --max-files 3000      höchstens so viele PDFs lesen
  --no-examples         nur Überblick und Merkmale, keine Textproben

Banken und Dokumentarten stehen in der Regeldatei (eine Quelle für das Skript und den Core, docs/dokument-zuordnung.md).
Danach: bericht.md durchsehen (es ist normaler Text) und weitergeben.
Voraussetzung: `pdftotext` (poppler), z. B. `brew install poppler`.
"""
import argparse
import collections
import datetime
import json
import os
import re
import shutil
import statistics
import subprocess
import sys

# ----------------------------------------------------------------------------- Banken und Dokumentarten

DEFAULT_RULES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "girocrab-core", "data", "document_rules.json")
BANKS = []  # (Name, Erkennungswörter, bankeigene Arten)
TYPES = []  # (Name, Muster), allgemeine Arten


def load_rules(path):
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        banks = [(b["name"], [k.lower() for k in b["erkennung"]],
                  [(t["name"], re.compile(t["muster"])) for t in b.get("arten", [])]) for b in data["banken"]]
        types = [(t["name"], re.compile(t["muster"])) for t in data["arten"]]
    except (OSError, ValueError, KeyError, re.error) as e:
        die(f"Regeldatei nicht lesbar ({path}): {e}")
    BANKS[:] = banks
    TYPES[:] = types


ISIN = re.compile(r"\b[A-Z]{2}[A-Z0-9]{9}[0-9]\b")
DATE = re.compile(r"\b\d{1,2}\.\d{1,2}\.(?:\d{4}|\d{2})\b")
AMOUNT = re.compile(r"(?<![\d.,])\d{1,3}(?:\.\d{3})*,\d{2}(?![\d])")
# mindestens 15 Zeichen: eine ISIN (12 Zeichen) ist keine IBAN
IBAN = re.compile(r"\b([A-Z]{2})\d{2}(?:[ ]?[A-Z0-9]{4}){3,7}(?:[ ]?[A-Z0-9]{1,4})?\b")
EMAIL = re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+")
PHONE = re.compile(r"(?:\+|00)\d[\d /()-]{7,}\d")
# Konto-, Depot-, Kunden- und Auftragsnummern: ab 7 Ziffern (eine Periode wie 10/2025 bleibt)
LONGNUM = re.compile(r"\d[\d ./-]{4,}\d")
PLZ_ORT = re.compile(r"\b\d{5}\s+[A-ZÄÖÜ][\wäöüß.-]+")
STREET = re.compile(r"(?i)\b[\wäöüß.-]*(?:straße|strasse|str\.|weg|allee|platz|gasse|ring|damm|ufer)\s*\d+[a-z]?\b")
SALUTATION = re.compile(r"(?i)\b(herr|frau|herrn|familie|eheleute)\b[^\n]*")


NAME_RUN = re.compile(r"\[Name\](?:[ ,]+[\wÄÖÜäöüß.-]+){0,3}?[ ,]*\[Name\]|\[Name\](?: [A-ZÄÖÜ][\wäöüß-]+)+|(?<![ ])(?:[A-ZÄÖÜ][\wäöüß-]+ )+\[Name\]")


def die(msg, code=1):
    print(msg, file=sys.stderr)
    sys.exit(code)


# ----------------------------------------------------------------------------- Schwärzen

class Redactor:
    def __init__(self, names):
        words = []
        for n in names:
            n = n.strip()
            if not n:
                continue
            words.append(n)
            words.extend(w for w in re.split(r"[\s,]+", n) if len(w) >= 3)
        # längste zuerst, damit "Max Mustermann" vor "Max" ersetzt wird
        words = sorted(set(words), key=len, reverse=True)
        self.name_re = re.compile("|".join(re.escape(w) for w in words), re.I) if words else None
        self.names = words

    def line(self, text):
        t = text
        t = EMAIL.sub("[Mail]", t)
        if self.name_re:
            t = self.name_re.sub("[Name]", t)
        t = SALUTATION.sub("[Anrede/Name]", t)
        t = PLZ_ORT.sub("[PLZ Ort]", t)
        t = STREET.sub("[Straße Nr]", t)
        t = IBAN.sub(lambda m: f"{m.group(1)}•• ••••", t)
        # Beträge, Daten und ISIN schützen, dann lange Zahlen und Telefon schwärzen
        keep = []

        def hold(m):
            keep.append(m.group(0))
            return f"\u0001{len(keep) - 1}\u0002"

        t = ISIN.sub(hold, t)
        t = DATE.sub(hold, t)
        t = AMOUNT.sub(hold, t)
        t = PHONE.sub("[Telefon]", t)
        t = LONGNUM.sub(lambda m: "#####" if len(re.sub(r"\D", "", m.group(0))) >= 7 else m.group(0), t)
        for i, v in enumerate(keep):
            t = t.replace(f"\u0001{i}\u0002", v)
        return NAME_RUN.sub("[Name]", t)

    def leaks(self, text):
        """Was im fertigen Bericht nicht stehen darf."""
        problems = []
        if IBAN.search(text):
            problems.append("IBAN")
        if EMAIL.search(text):
            problems.append("Mailadresse")
        if re.search(r"\[Name\] [A-ZÄÖÜ][a-zäöüß]{2,}|(?<![ ])[A-ZÄÖÜ][a-zäöüß]{2,} \[Name\]", text):
            ms = re.findall(r"\[Name\] [A-ZÄÖÜ][a-zäöüß]{2,}|(?<![ ])[A-ZÄÖÜ][a-zäöüß]{2,} \[Name\]", text)
            problems.append("Name neben dem Platzhalter [Name]: " + ", ".join(sorted({re.sub(r"[a-zäöüß]", "*", m) for m in ms})[:8]))
        for n in self.names:
            if len(n) >= 3 and re.search(re.escape(n), text, re.I):
                problems.append(f"Name „{n}“")
        return problems


# ----------------------------------------------------------------------------- Lesen

EXTRACTOR = []  # --extractor: command that prints the text of the PDF it is given


def pdf_text(path, pages=2):
    if EXTRACTOR:
        try:
            out = subprocess.run(EXTRACTOR + [path], capture_output=True, timeout=120)
        except (OSError, subprocess.TimeoutExpired):
            return ""
        return out.stdout.decode("utf-8", "replace") if out.returncode == 0 else ""
    try:
        out = subprocess.run(
            ["pdftotext", "-layout", "-l", str(pages), "-enc", "UTF-8", path, "-"],
            capture_output=True, timeout=60,
        )
    except (OSError, subprocess.TimeoutExpired):
        return ""
    if out.returncode != 0:
        return ""
    return out.stdout.decode("utf-8", "replace")


def page_count(path):
    try:
        out = subprocess.run(["pdfinfo", path], capture_output=True, timeout=30)
        m = re.search(rb"Pages:\s+(\d+)", out.stdout)
        return int(m.group(1)) if m else 0
    except (OSError, subprocess.TimeoutExpired):
        return 0


def guess_bank(path_lower, text_lower, top):
    hay = path_lower + " " + text_lower[:3000]
    for name, keys, _own in BANKS:
        if any(k in hay for k in keys):
            return name
    return f"unbekannt ({top})" if top else "unbekannt"


def guess_type(name_lower, head_lower, bank=""):
    hay = re.sub(r"[_]+", " ", name_lower) + " " + head_lower
    own = next((t for n, _k, t in BANKS if n == bank), [])
    for label, pattern in own + TYPES:  # bankeigene Arten zuerst
        if pattern.search(hay):
            return label
    return "Sonstiges"


def head_lines(text, n, redactor):
    lines = []
    for raw in text.splitlines():
        s = re.sub(r"\s{2,}", "  ", raw.strip())
        if not s:
            continue
        s = redactor.line(s)
        s = NAME_RUN.sub("[Name]", s)
        lines.append(s)
        if len(lines) >= n + 4:
            break
    # Adressblock: bis zu drei Zeilen vor einer PLZ-Zeile (Straße, Name, Zusatz) komplett ausblenden
    for i, l in enumerate(lines):
        if l.startswith("[PLZ Ort]"):
            for j in range(max(0, i - 3), i):
                if len(lines[j]) <= 60 and ":" not in lines[j] and not AMOUNT.search(lines[j]):
                    lines[j] = "[Adresszeile]"
    return lines[:n]


# ----------------------------------------------------------------------------- Bericht

def build(folder, args, redactor):
    files = []
    for root, dirs, names in os.walk(folder):
        dirs[:] = [d for d in dirs if not d.startswith(".")]
        for n in names:
            if n.lower().endswith(".pdf") and not n.startswith("."):
                files.append(os.path.join(root, n))
    files.sort()
    total = len(files)
    if len(files) > args.max_files:  # gleichmäßig über den ganzen Ordner verteilen, nicht nur die ersten
        step = len(files) / args.max_files
        files = [files[int(i * step)] for i in range(args.max_files)]

    groups = collections.defaultdict(list)  # (bank, type) -> list of dict
    no_text = 0
    for path in files:
        try:
            if os.path.getsize(path) > 40 * 1024 * 1024:
                continue
        except OSError:
            continue
        rel = os.path.relpath(path, folder)
        top = rel.split(os.sep)[0] if os.sep in rel else ""
        text = pdf_text(path)
        if len(text.strip()) < 40:
            no_text += 1
            continue
        low = text.lower()
        head = low[:900]
        bank = guess_bank(rel.lower(), low, redactor.line(top))
        if args.bank and not any(b.lower() in bank.lower() for b in args.bank):
            continue
        kind = guess_type(os.path.basename(path).lower(), head, bank)
        kind_text = guess_type("", head, bank)  # was der Inhalt allein sagt, ohne Dateiname
        lines = head_lines(text, args.lines, redactor)
        first_page = text.split("\f")[0]
        isins_head = ISIN.findall("\n".join(first_page.splitlines()[:25]))
        groups[(bank, kind)].append({
            "name": redactor.line(os.path.basename(path)),
            "mtime": os.path.getmtime(path),
            "pages": page_count(path),
            "chars": len(text),
            "isin_head": bool(isins_head),
            "isin_any": bool(ISIN.search(text)),
            "wkn": bool(re.search(r"\bWKN\b", text)),
            "dir_head": bool(re.search(r"(?i)\b(kauf|verkauf|sparplan|dividende|ausschüttung)\b", "\n".join(first_page.splitlines()[:25]))),
            "amounts": len(AMOUNT.findall(first_page)),
            "dates": len(DATE.findall(first_page)),
            "iban": bool(IBAN.search(text)),
            "labels": [bool(re.search(re.escape(l).replace("\\ ", r"\s+"), low)) for l in args.etikett],
            "iban_head": bool(IBAN.search("\n".join(first_page.splitlines()[:25]))),
            "acct_label": bool(re.search(r"(?i)(depot|konto|kunden)[- ]?(nr|nummer)", first_page)),
            "kind_text": kind_text,
            "same_by_text": kind_text == kind,
            "title_line": next((l for l in head_lines(text, 3, redactor)), ""),
            "lines": lines,
        })

    out = []
    out.append("# Probenbericht (lokal erzeugt, geschwärzt)\n")
    out.append(f"Erzeugt {datetime.date.today().isoformat()} · {total} PDFs im Ordner, "
               f"{sum(len(v) for v in groups.values())} ausgewertet, {no_text} ohne lesbaren Text (Scans).\n")
    out.append("Geschwärzt: IBAN, Namen, Adressen, Mail, Telefon, Zahlen ab 7 Ziffern. "
               "Stehen geblieben: Beträge, Daten, ISIN, Wertpapiernamen.\n")

    out.append("## Überblick\n")
    out.append("| Bank | Art | Anzahl | ISIN im Kopf | Richtung im Kopf | Beträge (Median) |")
    out.append("|---|---|---:|---:|---:|---:|")
    for (bank, kind) in sorted(groups):
        items = groups[(bank, kind)]
        n = len(items)
        isin = sum(i["isin_head"] for i in items)
        dirs = sum(i["dir_head"] for i in items)
        med = int(statistics.median(i["amounts"] for i in items))
        out.append(f"| {bank} | {kind} | {n} | {isin}/{n} | {dirs}/{n} | {med} |")
    out.append("")

    out.append("## Je Bank und Art\n")
    for (bank, kind) in sorted(groups):
        items = sorted(groups[(bank, kind)], key=lambda i: i["mtime"])
        n = len(items)
        out.append(f"### {bank} · {kind} (n={n})\n")
        pages = [i["pages"] for i in items if i["pages"]]
        out.append(f"- Seiten: {min(pages)}–{max(pages)}" if pages else "- Seiten: ?")
        out.append(f"- ISIN auf der ersten Seite: {sum(i['isin_any'] for i in items)}/{n} · "
                   f"WKN-Angabe: {sum(i['wkn'] for i in items)}/{n} · IBAN im Text: {sum(i['iban'] for i in items)}/{n}")
        same = sum(i["same_by_text"] for i in items)
        alt = collections.Counter(i["kind_text"] for i in items if not i["same_by_text"])
        out.append(f"- Art auch ohne Dateiname aus dem Text erkannt: {same}/{n}"
                   + (" · sonst als: " + ", ".join(f"{k} ×{c}" for k, c in alt.most_common(3)) if alt else ""))
        for k, label in enumerate(args.etikett):
            out.append(f"- Etikett „{label}“ im Text: {sum(i['labels'][k] for i in items)}/{n}")
        out.append(f"- IBAN in den ersten 25 Zeilen: {sum(i['iban_head'] for i in items)}/{n} · "
                   f"Zeile mit Depot-/Konto-/Kundennummer: {sum(i['acct_label'] for i in items)}/{n}")
        titles = collections.Counter(i["title_line"] for i in items)
        out.append("- Häufigste Anfangszeilen: " + " | ".join(f"„{t}“ ×{c}" for t, c in titles.most_common(3)))
        names = collections.Counter(re.sub(r"\d", "#", i["name"]) for i in items)
        out.append("- Dateinamen-Muster: " + " | ".join(f"`{t}` ×{c}" for t, c in names.most_common(3)))
        out.append("")
        if not args.no_examples:
            picks = items if n <= args.per_type else [items[0], items[n // 2], items[-1]][: args.per_type]
            for k, it in enumerate(picks, 1):
                out.append(f"**Beispiel {k}** (`{it['name']}`, {it['pages']} S., {it['chars']} Zeichen)\n")
                out.append("```text")
                out.extend(it["lines"])
                out.append("```\n")
    return "\n".join(out) + "\n"


def main():
    ap = argparse.ArgumentParser(description="Geschwärzter Strukturbericht über einen Ordner mit Bank-PDFs.")
    ap.add_argument("folder")
    ap.add_argument("--name", action="append", default=[], help="eigener Name (mehrfach möglich), wird geschwärzt")
    ap.add_argument("--out", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "bericht.md"))
    ap.add_argument("--per-type", type=int, default=3)
    ap.add_argument("--lines", type=int, default=18)
    ap.add_argument("--max-files", type=int, default=3000)
    ap.add_argument("--no-examples", action="store_true")
    ap.add_argument("--regeln", default=DEFAULT_RULES, help="Regeldatei (JSON)")
    ap.add_argument("--extractor", default="",
                    help="Befehl, der den Text einer PDF ausgibt, z. B. 'cargo run -q -p girocrab-core --example pdf_text --' "
                         "(dann zeigt der Bericht, was die App liest, nicht was pdftotext liest)")
    ap.add_argument("--etikett", action="append", default=[],
                    help="Etikett, das im Text vorkommen soll (z. B. 'zu ihren lasten vor steuern'), mehrfach möglich")
    ap.add_argument("--bank", action="append", default=[], help="nur diese Bank(en), Teil des Namens genügt (mehrfach möglich)")
    args = ap.parse_args()

    folder = os.path.abspath(os.path.expanduser(args.folder))
    if not os.path.isdir(folder):
        die(f"Ordner nicht gefunden: {folder}")
    if args.extractor:
        import shlex
        EXTRACTOR[:] = shlex.split(args.extractor)
    elif not shutil.which("pdftotext"):
        die("pdftotext fehlt. Installieren mit: brew install poppler")
    if not args.name:
        print("Hinweis: ohne --name werden Namen nur an Anreden und Adresszeilen erkannt.", file=sys.stderr)

    load_rules(args.regeln)
    redactor = Redactor(args.name)
    report = build(folder, args, redactor)
    problems = redactor.leaks(report)
    if problems:
        die("Abbruch, es wurde nichts geschrieben. Im Bericht steht noch: " + ", ".join(sorted(set(problems))) +
            "\nMit weiteren --name-Angaben oder --no-examples erneut versuchen.")
    out = os.path.abspath(args.out)
    if os.path.commonpath([out, folder]) == folder:
        die("Die Zieldatei liegt im ausgewerteten Ordner. Bitte einen anderen Ort wählen (--out).")
    with open(out, "w", encoding="utf-8") as f:
        f.write(report)
    print(f"Bericht geschrieben: {out}")


if __name__ == "__main__":
    main()
