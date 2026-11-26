# UI-Audits: kleine Prüfungen einzelner Ansichten

Ein UI-Audit prüft **eine** Ansicht und macht daraus wenige, gut abgrenzbare Aufgaben. Ziel ist nicht, die ganze
App auf einmal zu bewerten, sondern Schritt für Schritt nachvollziehbare Verbesserungen zu sammeln. Hintergrund
und Diskussion: RFC #1673; erster Pilot: [Buchungsinspektor](ui-audits/buchungen.md#buchungsinspektor).
Prüfstand und offene Nachprüfungen stehen in der [Audit-Übersicht](ui-audits/README.md).

Werkzeug ist [Impeccable](#mit-impeccable). Maßstab sind [`PRODUCT.md`](../PRODUCT.md) (wofür und für wen) und
[`DESIGN.md`](../DESIGN.md) (wie es heute aussieht, mit den GiroCrab-Regeln unter „Do's and Don'ts“); Impeccable
liest beide. Technische Regeln stehen in [`CLAUDE.md`](../CLAUDE.md).

## Ablauf

1. **Ansicht und Stand festlegen.** Pro Audit eine Ansicht, ihren Einstieg in der App und die Hauptdateien
   festhalten. In der [Audit-Übersicht](ui-audits/README.md) und auf GitHub nach dem Namen der Ansicht sowie
   nach `[UI-Audit]` in Issues und PRs suchen. Vorhandene Berichte fortschreiben. Den geprüften Commit mit
   `git rev-parse --short HEAD` erfassen. Patches mit der vollständigen Commit-SHA oder einem verlinkten Diff
   festhalten. Das xcframework aus dem Prüfstand bauen. Hat der Core einen anderen Stand, seinen Commit und
   etwaige lokale Änderungen ebenfalls dokumentieren.
2. **Demo-Fall festlegen.** Startargumente, Konto oder Buchung, Klickfolge und Vergleichszustände vor dem
   Durchgang festhalten. Bei relativen Zeiträumen auch das Datum der Aufnahme nennen. Bei einem Sheet den
   Elternbildschirm und den Auslöser nennen. Für Dialoge und Fehlermeldungen simulierte Fälle mit Demo-Daten verwenden. Die [Prüfmatrix](#prüfmatrix) durchgehen und
   pro Plattform Betriebssystem, Gerät und Abweichungen festhalten.
3. **Impeccable ausführen.** Native `critique` und `audit` auf die Hauptdatei der Ansicht anwenden, mit
   `PRODUCT.md` und `DESIGN.md` als Kontext. Dazu die [GiroCrab-Regeln](#was-impeccable-nicht-kennt) prüfen.
   Die Bewertungen, den geprüften Zustand und Einschränkungen beim Lauf dokumentieren.
4. **Befunde bestätigen.** Code-Stelle und Demo-Aufnahme für jeden Befund belegen. Auf macOS und iPhone
   prüfen, sofern die Ansicht dort verfügbar ist. Für iPad-spezifische Befunde das passende Gerät ergänzen.
   Im Bericht die Startargumente, Interaktionen sowie erwartetes und beobachtetes
   Ergebnis nennen. Dem jeweiligen Prüfnachweis einen `Demo-Einstieg` mit Startargumenten oder Bedienfolge geben.
   Fälle ohne Nachweis als ausstehende Prüfung in die Übersicht aufnehmen.
5. **Bericht schreiben.** Einen vorhandenen Themenbericht in `docs/ui-audits/` ergänzen, wenn die Ansicht
   denselben Arbeitsablauf oder wichtige Komponenten teilt. Für einen neuen Bereich die
   [Vorlage](#vorlage-für-den-bericht) verwenden. Jede Ansicht erhält eine eigene Zeile in der Bewertung
   und einen gleichnamigen Abschnitt im Prüfnachweis. Jeder bestätigte Befund bekommt eine bleibende ID
   aus Ansichtskürzel und Nummer, z. B. `INS-1`. Neue Befunde bekommen die nächste freie Nummer.
   Die Tabelle fasst jeden Befund in einer Zeile zusammen. Unter „Prüfnachweis“ reichen die Schritte,
   die speziell für diese Ansicht gelten. Die allgemeine Prüfmatrix steht hier in der Anleitung.
6. **Sortieren.** Jeder Befund bekommt genau einen Weg:
   - **Fix:** klein und unstrittig → direkt im Audit-PR beheben.
   - **Issue:** klar, aber eigene Arbeit → höchstens fünf Issues pro Audit, Titel
     `[UI-Audit] <Bereich>: <Beobachtung>`, ggf. `good first issue` / `help wanted`.
   - **Entscheidung:** betrifft mehrere Ansichten (gemeinsame Abstände, Radien, Betragsgrößen) →
     bleibt im Bericht und wird im RFC-Issue zur Diskussion gestellt, nicht nebenbei geändert.
   - **Verworfen:** nach Prüfung kein Problem → mit Begründung im Bericht lassen.
7. **Nachprüfen und abschließen.** Den Demo-Fall bei einem Fix im selben Zustand auf dem Ausgangsstand und
   auf dem Fix-Stand wiederholen. Datum, Commit, Plattform, Ergebnis und Beleg im Bericht ergänzen. Im PR die
   IDs der behobenen Befunde nennen und Vorher/Nachher-Aufnahmen zuordnen. Der Bericht verlinkt
   den zuständigen PR oder das Issue und führt pro Befund genau einen Status:
   `open` für offene Befunde (auch teilweise behobene), `fixed` für vollständig behobene und nachgeprüfte
   Befunde, deren Fix auf `main` liegt oder im selben PR mitgeliefert wird,
   `decision` für ausstehende gemeinsame Entscheidungen und `dismissed` für verworfene Befunde.
   Ändert sich `main` an der Ansicht, bevor der PR gemergt ist:
   die betroffenen Fälle auf dem neuen Stand erneut prüfen. Die Übersicht aktualisieren und
   `scripts/lint-ui-audits.sh` ausführen. Der lokale Check prüft die Vorlage und die Berichte. Eine kurze Zusammenfassung
   kommt bei Veröffentlichung als Kommentar ins RFC-Issue.

## Stand und Wiederholungen

- **„Datum / Stand“ je Ansicht.** Die Bewertungszeile enthält das Prüfdatum (`TT.MM.JJJJ`) und den geprüften
  App-Commit des Erstaudits oder letzten vollständigen Audits. Datum und Commit gehören zu den Scores
  dieser Zeile. Textkorrekturen, Links und reine Statuspflege lassen beide Angaben bestehen.
- **Gezielte Nachprüfung.** Nach einem Fix, einem Verdacht auf Regression oder einer kleinen Änderung
  die betroffenen Demo-Fälle auf dem aktuellen Stand wiederholen. Unter „Prüfnachweis“ eine Zeile mit Datum,
  geprüftem Commit, Befund-IDs und Plattformen ergänzen. Ergebnis und Beleg den geprüften IDs zuordnen.
  Wenn der neue Durchgang dieselben Fälle abdeckt, den bisherigen Nachweis ersetzen. Datum und Scores
  gelten weiter für den vollständigen Durchgang. Tritt ein behobener Befund erneut auf, seine ID mit `open` wieder öffnen.
- **Vollständiges Audit wiederholen.** Wenn sich Aufgabe, Aufbau, Bedienung, Plattformverhalten oder
  gemeinsame Komponenten so ändern, dass mehrere Prüffälle betroffen sind oder die Bewertung fraglich wird,
  den [Ablauf](#ablauf) am aktuellen `main` vollständig durchlaufen. Die bisherigen Demo-Fälle aus der
  Prüfmatrix wiederholen und die neuen Zustände ergänzen. Im selben Bericht Bewertungszeile und
  Prüfnachweis dieser Ansicht aktualisieren. Bestehende Befund-IDs erhalten und neue fortlaufend ergänzen.
  Frühere Bewertungen bleiben in der Git-Historie nachvollziehbar.

Eine fällige Nachprüfung mit Anlass in der [Audit-Übersicht](ui-audits/README.md) vormerken. Das Datum in
ihrem Kopf bezeichnet den letzten Abgleich der Übersicht mit der App. Eine neue Version allein löst
keinen neuen Durchgang aus.

## Prüfmatrix

Die Standardaufnahme dient auf jeder Plattform als Referenz. Für alle Varianten dieselben Demo-Daten, Filter,
Sprache und Auswahl verwenden. Die Mindestbreite des Fensters kann die verlangte Breite überschreiben. Dann die
tatsächlich erreichte Breite festhalten.

| Umgebung | Zustände |
|---|---|
| macOS | Hell und dunkel, „Sehr groß“, schmale Fensterbreite, `--redact` und bei eigenen Kategorien, Gruppen oder Wertpapieren `--redact=labels`. Tastaturpfad und Fokus bei interaktiven Ansichten prüfen. |
| iPhone | Hell und dunkel, „Accessibility XL“, `--redact` bzw. `--redact=labels`. Bei Sheets Öffnen, Schließen und Platz für die Inhalte prüfen. |
| iPad | Prüfen, wenn sich Navigation, Spaltenaufteilung oder Darstellung von iPhone unterscheiden. Größe und Ausrichtung nennen. |
| Zustände der Ansicht | Soweit vorhanden Auswahl, leere Ansicht oder keine Treffer, Fehler samt nächstem Schritt und „Bewegung reduzieren“ bei sichtbaren Übergängen. |

Im Bericht zu jeder geprüften oder ausstehenden Variante kurz das Ergebnis notieren. Bei einem Fix denselben
Fall einschließlich Fenstergröße, Schriftstufe, Farben und Schwärzung vor und nach der Änderung aufnehmen.

## Mit Impeccable

[Impeccable](https://impeccable.style/docs/) ist ein Agent-Skill für Design-Reviews (Installation laut
Projektseite, lokal; der Skill wird nicht ins Repository eingebettet). Er liest `PRODUCT.md` und `DESIGN.md`
aus dem Projektordner und legt seine Berichte unter `.impeccable/` ab (von Git ignoriert).

```text
/impeccable critique GiroCrabApp/Sources/GiroCrabUI/<Datei>.swift
/impeccable audit GiroCrabApp/Sources/GiroCrabUI/<Datei>.swift
```

- `critique`: Design-Bewertung nach zehn Heuristiken (Score von 40, nicht anwendbare als n/a), kognitive Last,
  Personas (u. a. eine aus `PRODUCT.md` › Users), 3–5 priorisierte Probleme (P0–P3).
- `audit`: technische Prüfung für native Apps in fünf Dimensionen (Barrierefreiheit, Performance, Theming,
  Plattformtreue, Anpassung; Score von 20) gegen die Apple-Richtlinien.
- Für native Apps gibt es keinen automatischen Detector (er liest nur Web-Code). Impeccable liefert hier die
  Methode; die Beobachtungen macht der Agent im Code und in Screenshots. Der Bericht sagt das so.
- Die Ausgabe ist ein Vorschlag. Der Agent ändert im Audit keinen Code; Fixes folgen erst nach dem Sortieren.

## Was Impeccable nicht kennt

Impeccable prüft Schriftgröße, Kontrast in hell und dunkel, „Bewegung reduzieren“, Trefferflächen und
Plattformtreue selbst. Diese GiroCrab-Regeln stehen zwar in `DESIGN.md`, werden aber zusätzlich von Hand geprüft:

| Regel | Prüfen |
|---|---|
| Schwärzung für „Problem melden“ | Screenshot mit `--redact`: Namen, IBANs, Zwecke, Notizen und Beträge verdeckt, auch in Titeln (`navigationTitle`, `.help`) |
| Beträge | über `Money.format` / `Money.signed`, `monospacedDigit`, `.fixedSize()`; nie gekürzt |
| Text der App folgt der Schriftgröße | `.appFont` statt `.font(.system(…))`; feste Maße mit `textScale` |
| Nur Demo-Daten | Screenshots nur mit `--demo-seed` / `GIROCRAB_DEMO_SEED=1` |

Schnelle Code-Hinweise (Treffer sind Kandidaten, keine Befunde):

```bash
cd GiroCrabApp/Sources/GiroCrabUI
TARGET=BookingDetailView.swift   # bei anderen Ansichten ersetzen
rg -n '\.font\(\.system\(' "$TARGET"                         # feste Schriftgröße
rg -n 'frame\((width|height):|minHeight:|minimum:' "$TARGET"   # feste Maße mit textScale?
rg -n 'navigationTitle|\.help\(' "$TARGET"                    # sensible Parametertexte
rg -n 'withAnimation\(|\.animation\(' "$TARGET"              # Bewegung reduzieren über Motion?
```

## Screenshots

Immer mit Demo-Daten und ohne Banner aufnehmen. In der Demo-App unter „Einstellungen › Allgemein ›
Mitteilungen“ die Projektmitteilungen ausstellen. `--screenshot` blendet den Demo-Hinweis aus. `--pick <Text>`
wählt die erste Buchung, deren Händler oder Gegenpartei den Text enthält, `--select <Konto>` ein Konto.
Screenshots im PR oder Issue anhängen und dort nach Befund-ID beschriften. Im Bericht beim jeweiligen Befund
den PR, das Issue oder den Bildlink angeben, damit der Beleg über den Dateinamen hinaus auffindbar bleibt.

Für Fixes gehören **Vorher/Nachher-Paare** in den PR: dieselbe Ansicht mit denselben Einstellungen, „nachher“ mit dem
Stand des Branches, „vorher“ aus `main` in einem eigenen Worktree. Das xcframework im Worktree aus dem
Ausgangsstand bauen. Zum Wiederholen eines früheren Audits `BASE_SHA` auf den dokumentierten Commit setzen.

```bash
BASE_SHA=$(git rev-parse upstream/main)   # diese SHA im Bericht erfassen
BASE_DIR=$(mktemp -d /tmp/girocrab-ui-base.XXXXXX)
git worktree add --detach "$BASE_DIR" "$BASE_SHA"
(cd "$BASE_DIR" && ./build-xcframework.sh) # macOS und iOS, für macOS allein: --macos-only
(cd "$BASE_DIR/GiroCrabApp" && swift build -q && \
  .build/debug/GiroCrabMac --demo-seed --pick rewe --light --text-size standard --screenshot /tmp/vorher-standard.png)
# iPhone: denselben Worktree unter "$BASE_DIR/GiroCrabApp/Xcode" bauen (siehe unten).
# Erst nach allen Aufnahmen den Worktree entfernen.
git worktree remove "$BASE_DIR"
```

### macOS

Aus `GiroCrabApp/` nach `swift build -q` (`--screenshot` blendet den Demo-Hinweis aus):

```bash
APP=.build/debug/GiroCrabMac
"$APP" --demo-seed --pick rewe --screenshot /tmp/audit-standard.png --light --text-size standard
"$APP" --demo-seed --pick rewe --screenshot /tmp/audit-larger.png --light --text-size larger
"$APP" --demo-seed --pick rewe --screenshot /tmp/audit-dark.png --dark --text-size standard
"$APP" --demo-seed --pick rewe --screenshot /tmp/audit-redact.png --light --redact
"$APP" --demo-seed --pick rewe --screenshot /tmp/audit-narrow.png --light --window-width 900
"$APP" --demo-seed --pick rewe --screenshot /tmp/audit-tall.png --light --window-height 1900
```

### iPhone

Auf dem iPhone schreibt `--screenshot` kein Bild. Es blendet den Demo-Hinweis aus, damit `simctl` das Bild
aufnehmen kann. `--pick` öffnet auf dem iPhone das Buchungs-Sheet. Das Beispiel in Bash läuft aus
`GiroCrabApp/Xcode/`, sobald das xcframework für iOS vorliegt. `simctl list devices booted` zeigt die IDs der
laufenden Geräte. Die separate Bundle-ID isoliert die Demo-App. Die Simulator-Einstellungen werden beim
Verlassen des Skripts wiederhergestellt. Der temporäre Xcode-Buildordner wird auch bei einem Fehler entfernt.

```bash
(
set -e
UDID='<ID des iPhone-Simulators>'
APP_ID=de.girocrab.app.uiaudit
OLD_APPEARANCE=$(xcrun simctl ui "$UDID" appearance)
OLD_SIZE=$(xcrun simctl ui "$UDID" content_size)
cleanup() {
  xcrun simctl terminate "$UDID" "$APP_ID" >/dev/null 2>&1 || true
  xcrun simctl ui "$UDID" appearance "$OLD_APPEARANCE" || true
  xcrun simctl ui "$UDID" content_size "$OLD_SIZE" || true
  rm -rf -- "$DD"
}
DD=$(mktemp -d /tmp/girocrab-ui-audit.XXXXXX)
trap cleanup EXIT
xcodebuild -quiet -project GiroCrab.xcodeproj -scheme GiroCrab -destination "id=$UDID" \
  -derivedDataPath "$DD" PRODUCT_BUNDLE_IDENTIFIER="$APP_ID" CODE_SIGNING_ALLOWED=NO build
xcrun simctl install "$UDID" "$DD/Build/Products/Debug-iphonesimulator/GiroCrab.app"
# Vor der Aufnahme einmal als Demo starten und wie oben beschrieben Mitteilungen ausstellen.
SIMCTL_CHILD_GIROCRAB_DEMO_SEED=1 xcrun simctl launch "$UDID" "$APP_ID" --demo-seed
read -r -p 'Mitteilungen in der Demo-App ausstellen, dann Eingabetaste drücken. ' _

shot() {  # shot <Datei> [Argumente …]
  xcrun simctl terminate "$UDID" "$APP_ID" >/dev/null 2>&1 || true
  SIMCTL_CHILD_GIROCRAB_DEMO_SEED=1 xcrun simctl launch "$UDID" "$APP_ID" --demo-seed \
    --screenshot /dev/null "${@:2}" >/dev/null
  sleep 15
  xcrun simctl io "$UDID" screenshot "$1"
}
xcrun simctl ui "$UDID" appearance light
xcrun simctl ui "$UDID" content_size large
shot /tmp/ios-standard.png --pick rewe
shot /tmp/ios-redact.png --pick rewe --redact
xcrun simctl ui "$UDID" appearance dark
shot /tmp/ios-dark.png --pick rewe
xcrun simctl ui "$UDID" appearance light
xcrun simctl ui "$UDID" content_size accessibility-extra-large
shot /tmp/ios-larger.png --pick rewe
)
```

Jede Aufnahme ansehen. Ist noch der Startbildschirm sichtbar, die App fertig laden lassen und `simctl io …
screenshot` ohne Neustart wiederholen. Wo ein Schritt Tippen oder Scrollen braucht, den Zustand vor der Aufnahme
herstellen und die genaue Folge im Bericht notieren.

## Vorlage für den Bericht

```markdown
# UI-Audit: <Themenbereich>

- **Ansicht:** <was genau, auf welchen Plattformen>
- **Dateien:** `GiroCrabApp/Sources/GiroCrabUI/<Datei>.swift`
- **Werkzeug:** Impeccable `critique` und `audit` (nativ), dazu GiroCrab-Regeln von Hand
- **Nachweis:** <Plattformen, geprüfte Varianten und Demo-Screenshots>

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| XYZ-1 | P1 | Barrierefreiheit | macOS | … | Fix | `fixed` |
| XYZ-2 | P2 | Konsistenz | macOS, iOS | … | Issue | `open` |
| XYZ-3 | P2 | Konsistenz | macOS, iOS | … | Entscheidung | `decision` |
| XYZ-4 | – | Datenschutz | macOS, iOS | … | Verworfen | `dismissed` |

## Impeccable-Bewertung

| Ansicht | Datum / Stand | Critique | Technisches Audit | Kurzbewertung |
|---|---|---|---|---|
| Beispielansicht | 25.11.2026, `main` @ `09f11987` | 25/36 | 14/20 | <Wichtigstes Ergebnis> |

## Prüfnachweis

### Beispielansicht

- **Nachprüfung:** <Datum, Commit, IDs, Plattformen, Ergebnis und Beleg, falls vorhanden>
- **Umgebung:** <Plattformen, Versionen, Geräte, Sprache, geprüfte Varianten und Abweichungen von der Prüfmatrix>
- **Build:** <fester Patch-Commit oder Diff und abweichender Core-Stand, falls verwendet>
- **Demo-Einstieg:** <Startargumente, Auswahl und Bedienfolge>
- **XYZ-2:** <Code-Stelle, Ergebnis, erwartetes Verhalten, Beleg und ggf. Issue oder PR>
```

Wenn die Prüfung keine Befunde ergibt, bleibt die Tabelle leer und darunter steht „Keine Befunde.“ In älteren
Audits steht `–` in der Spalte „Priorität“, wenn der ursprüngliche Durchgang keine Einstufung festgehalten hat.
