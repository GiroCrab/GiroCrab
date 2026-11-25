#!/usr/bin/env bash
# Validate report structure and per-view audit records. Review the evidence separately.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

if [ "$#" -eq 0 ]; then
  set -- docs/ui-audits/*.md
fi

findings='| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |'
ratings='| Ansicht | Datum / Stand | Critique | Technisches Audit | Kurzbewertung |'
for header in "$findings" "$ratings"; do
  grep -Fxq "$header" docs/ui-audit.md || { echo "docs/ui-audit.md: Tabelle der Vorlage fehlt: $header" >&2; exit 1; }
done

failed=0
for report in "$@"; do
  [[ "$(basename "$report")" != README.md ]] || continue
  awk -v findings="$findings" -v ratings="$ratings" '
    function fail(message) {
      print FILENAME ":" FNR ": " message
      errors++
    }
    function trim(value) {
      sub(/^[[:space:]]+/, "", value)
      sub(/[[:space:]]+$/, "", value)
      return value
    }
    function valid_date(value,    parts, day, month, year, days) {
      split(substr(value, 1, 10), parts, "[.]")
      day = parts[1] + 0
      month = parts[2] + 0
      year = parts[3] + 0
      split("31 28 31 30 31 30 31 31 30 31 30 31", days, " ")
      if (year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)) days[2] = 29
      return year > 0 && month >= 1 && month <= 12 && day >= 1 && day <= days[month]
    }
    function valid_score(value, maximum,    parts, limit) {
      if (value !~ /^[0-9]+\/[0-9]+$/) return 0
      split(value, parts, "/")
      limit = parts[2] + 0
      if (maximum == 20) return limit == 20 && parts[1] + 0 <= limit
      return limit >= 4 && limit <= maximum && limit % 4 == 0 && parts[1] + 0 <= limit
    }
    BEGIN {
      split("Ansicht|Dateien|Werkzeug|Nachweis", metadata, "[|]")
      split("Befunde|Impeccable-Bewertung|Prüfnachweis", sections, "[|]")
      header[1] = findings
      header[2] = ratings
      separator[1] = "|---|---|---|---|---|---|---|"
      separator[2] = "|---|---|---|---|---|"
      fields[1] = 7
      fields[2] = 5
    }
    FNR == 1 && $0 !~ /^# UI-Audit: .+/ { fail("Titel muss mit # UI-Audit: beginnen") }
    /^- \*\*/ && section == 0 {
      prefix = "- **" metadata[++field] ":** "
      if (index($0, prefix) != 1 || trim(substr($0, length(prefix) + 1)) == "") fail("Metadaten erwartet: " metadata[field])
    }
    /^## / {
      if ($0 != "## " sections[++section]) fail("Abschnitt erwartet: " sections[section])
    }
    expect_separator {
      if ($0 == separator[expect_separator]) { in_table = expect_separator; expect_separator = 0; next }
      fail("Trennzeile erwartet: " separator[expect_separator])
      expect_separator = 0
    }
    section in header && $0 == header[section] { tables[section]++; expect_separator = section; next }
    section == 1 && $0 == "Keine Befunde." { empty = 1 }
    in_table && $0 !~ /^[|]/ { in_table = 0 }
    section in header && !in_table && /^[[:space:]]*[|]/ {
      fail("Tabellenzeile außerhalb der Tabelle")
      next
    }
    in_table && /^[|]/ {
      row = $0
      gsub(/\\[|]/, "PIPE", row)
      count = split(row, cell, "[|]")
      if (count != fields[in_table] + 2 || trim(cell[count]) != "") { fail("Falsche Spaltenzahl"); next }

      if (in_table == 1) {
        id = trim(cell[2])
        if (id !~ /^[A-Z][A-Z0-9]*-[1-9][0-9]*$/ || seen[id]++) fail("Ungültige oder doppelte Befund-ID: " id)
        if (trim(cell[3]) !~ /^(P[0-3]|–)$/) fail(id ": Priorität muss P0–P3 oder – sein")
        if (trim(cell[8]) !~ /^`(open|fixed|decision|dismissed)`$/) fail(id ": Status muss open, fixed, decision oder dismissed sein")
        entries++
      } else {
        view = trim(cell[2])
        if (view == "" || view in scored) fail("Fehlende oder doppelte Ansicht: " view)
        scored[view] = 1
        stamp = trim(cell[3])
        split(stamp, stamp_fields, "`")
        commit_length = length(stamp_fields[4])
        if (stamp !~ /^[0-9][0-9]\.[0-9][0-9]\.[0-9][0-9][0-9][0-9], `[^`]+` @ `[a-f0-9]+`$/ ||
            !valid_date(stamp) || commit_length < 7 || commit_length > 40) fail(view ": Gültiges Prüfdatum und Commit erwartet")
        if (!valid_score(trim(cell[4]), 40) || !valid_score(trim(cell[5]), 20)) fail(view ": Gültige Critique- und Audit-Scores erwartet")
      }
    }
    section == 3 && /^### / {
      view = trim(substr($0, 5))
      if (view == "" || view in evidence) fail("Fehlender oder doppelter Name im Prüfnachweis: " view)
      evidence[view] = 0
    }
    section == 3 && view in evidence && /^- \*\*Demo-Einstieg:\*\* [^[:space:]]/ {
      evidence[view] = 1
    }
    END {
      if (field != 4) fail("Vier Metadatenfelder erwartet")
      if (section != 3) fail("Drei Abschnitte erwartet")
      if (tables[1] != 1 || tables[2] != 1 || expect_separator) fail("Je eine Befund- und Bewertungstabelle erwartet")
      if (!entries && !empty) fail("Befunde oder den Hinweis Keine Befunde. ergänzen")
      if (entries && empty) fail("Keine Befunde. passt nicht zu einer ausgefüllten Tabelle")
      for (view in scored) {
        score_count++
        if (!(view in evidence)) fail("Prüfnachweis fehlt: " view)
        else if (!evidence[view]) fail("Demo-Einstieg im Prüfnachweis fehlt: " view)
      }
      for (view in evidence) {
        if (!(view in scored)) fail("Bewertung fehlt: " view)
      }
      if (!score_count) fail("Mindestens eine Bewertung erwartet")
      exit (errors != 0)
    }
  ' "$report" >&2 || failed=1
done
exit "$failed"
