#!/usr/bin/env bash
# Validate the shared report format. Evidence and the demo steps are reviewed in the audit.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

if [ "$#" -eq 0 ]; then
  set -- docs/ui-audits/*.md
fi

header='| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |'
grep -Fxq "$header" docs/ui-audit.md || { echo "docs/ui-audit.md: Befundtabelle der Vorlage fehlt" >&2; exit 1; }

failed=0
for report in "$@"; do
  [[ "$(basename "$report")" != README.md ]] || continue
  awk -v header="$header" '
    function fail(message) {
      print FILENAME ":" FNR ": " message
      errors++
    }
    function trim(value) {
      sub(/^[[:space:]]+/, "", value)
      sub(/[[:space:]]+$/, "", value)
      return value
    }
    BEGIN {
      split("Datum / Stand|Ansicht|Dateien|Werkzeug|Nachweis", metadata, "[|]")
      split("Befunde|Impeccable-Bewertung|Prüfnachweis", sections, "[|]")
    }
    FNR == 1 && $0 !~ /^# UI-Audit: .+/ { fail("Titel muss mit # UI-Audit: beginnen") }
    /^- \*\*/ && section == 0 {
      if (index($0, "- **" metadata[++field] ":** ") != 1) fail("Metadaten erwartet: " metadata[field])
    }
    /^## / {
      if ($0 != "## " sections[++section]) fail("Abschnitt erwartet: " sections[section])
    }
    expect_separator {
      expect_separator = 0
      if ($0 == "|---|---|---|---|---|---|---|") { in_table = 1; next }
      fail("Trennzeile mit sieben Spalten erwartet")
    }
    section == 1 && $0 == header { tables++; expect_separator = 1; next }
    section == 1 && $0 == "Keine Befunde." { empty = 1 }
    in_table && $0 !~ /^[|]/ { in_table = 0 }
    in_table && /^[|]/ {
      row = $0
      gsub(/\\[|]/, "PIPE", row)
      count = split(row, cell, "[|]")
      if (count != 9 || trim(cell[9]) != "") { fail("Befund braucht sieben Spalten"); next }
      id = trim(cell[2])
      if (id !~ /^[A-Z][A-Z0-9]*-[1-9][0-9]*$/ || seen[id]++) fail("Ungültige oder doppelte Befund-ID: " id)
      if (trim(cell[3]) !~ /^(P[0-3]|–)$/) fail(id ": Priorität muss P0–P3 oder – sein")
      if (trim(cell[8]) !~ /^`(open|fixed|decision|dismissed)`$/) fail(id ": Status muss open, fixed, decision oder dismissed sein")
      findings++
    }
    END {
      if (field != 5) fail("Fünf Metadatenfelder erwartet")
      if (section != 3) fail("Drei Abschnitte erwartet")
      if (tables != 1) fail("Genau eine Befundtabelle erwartet")
      if (!findings && !empty) fail("Befunde oder den Hinweis Keine Befunde. ergänzen")
      if (findings && empty) fail("Keine Befunde. passt nicht zu einer ausgefüllten Tabelle")
      exit (errors != 0)
    }
  ' "$report" >&2 || failed=1
done
exit "$failed"
