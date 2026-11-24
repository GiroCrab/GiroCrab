# UI-Audits: kleine Prüfungen einzelner Ansichten

Ein UI-Audit prüft **eine** Ansicht und macht daraus wenige, gut abgrenzbare Aufgaben. Ziel ist nicht, die ganze
App auf einmal zu bewerten, sondern Schritt für Schritt nachvollziehbare Verbesserungen zu sammeln. Hintergrund
und Diskussion: RFC #1673; erster Pilot: [`docs/ui-audits/buchungs-inspektor.md`](ui-audits/buchungs-inspektor.md).

Werkzeug ist [Impeccable](#mit-impeccable). Maßstab sind [`PRODUCT.md`](../PRODUCT.md) (wofür und für wen) und
[`DESIGN.md`](../DESIGN.md) (wie es heute aussieht, mit den GiroCrab-Regeln unter „Do's and Don'ts“); Impeccable
liest beide. Technische Regeln stehen in [`CLAUDE.md`](../CLAUDE.md).

## Ablauf

1. **Ansicht wählen.** Ein Bereich pro Audit, z. B. „Buchungs-Inspektor“ oder „Seitenleiste“, mit den Dateien,
   die ihn bauen. Vorher in den offenen Issues mit `[UI-Audit]` nachsehen, ob schon jemand daran arbeitet.
2. **Impeccable ausführen:** `critique` und `audit` auf die Hauptdatei der Ansicht (siehe unten). Den Skill
   wirklich laufen lassen; ein Durchgang „im Sinne von“ Impeccable ersetzt ihn nicht.
3. **Ergänzend prüfen,** was Impeccable nicht kennt: [GiroCrab-Regeln](#was-impeccable-nicht-kennt).
4. **Bestätigen.** Jede Beobachtung im Code und in Screenshots mit Demo-Daten nachweisen, auf **macOS und
   iPhone** ([Screenshots](#screenshots)). Was sich nicht bestätigen lässt, kommt nicht in den Bericht.
5. **Bericht schreiben** in `docs/ui-audits/<bereich>.md` ([Vorlage](#vorlage-für-den-bericht)). Jede Beobachtung
   bekommt eine ID aus Kürzel und Nummer, z. B. `INS-1`; der Kopf nennt Werkzeug und Scores.
6. **Sortieren.** Jedes Finding bekommt genau einen Weg:
   - **Fix:** klein und unstrittig → direkt im Audit-PR beheben.
   - **Issue:** klar, aber eigene Arbeit → höchstens fünf Issues pro Audit, Titel
     `[UI-Audit] <Bereich>: <Beobachtung>`, ggf. `good first issue` / `help wanted`.
   - **Entscheidung:** betrifft mehrere Ansichten (gemeinsame Abstände, Radien, Betragsgrößen) →
     bleibt im Bericht und wird im RFC-Issue zur Diskussion gestellt, nicht nebenbei geändert.
   - **Verworfen:** nach Prüfung kein Problem → mit Begründung im Bericht lassen.
7. **Nachvollziehbar halten.** Commit-Nachricht und PR nennen die IDs der behobenen Findings; der Bericht nennt
   für jedes Finding den Status bzw. das Issue. Ändert sich `main` an der Ansicht, bevor der PR gemergt ist:
   neu aufsetzen und Findings, Zeilenangaben und Screenshots erneut prüfen. Eine kurze Zusammenfassung kommt als
   Kommentar ins RFC-Issue.

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
grep -n '\.font(\.system(' <Datei>.swift                    # Text, der der Schriftgröße nicht folgt
grep -n 'frame(width:\|frame(height:\|minHeight:\|minimum:' <Datei>.swift   # feste Maße: mit textScale?
grep -n 'navigationTitle\|\.help(' <Datei>.swift            # Parametertexte: sensible Inhalte ersetzt?
grep -n 'withAnimation(\|\.animation(' <Datei>.swift | grep -v Motion   # Animation ohne „Bewegung reduzieren“
```

## Screenshots

Immer mit Demo-Daten und ohne Banner (Demo-Hinweis, Mitteilungen wie die FinTS-Registrierung). `--pick <Text>`
wählt die erste Buchung, deren Händler oder Gegenpartei den Text enthält, `--select <Konto>` ein Konto.
Screenshots gehören in den PR oder das Issue, nicht ins Repository.

Für Fixes gehören **Vorher/Nachher-Paare** in den PR: dieselbe Ansicht mit denselben Einstellungen, „nachher“ mit dem
Stand des Branches, „vorher“ aus `main` in einem eigenen Worktree, damit im Branch nichts überschrieben wird. Der
Worktree nutzt das xcframework des Branches mit (bei UI-Audits ist der Core unverändert):

```bash
git worktree add /tmp/girocrab-main upstream/main
ln -s "$PWD/generated" /tmp/girocrab-main/generated
(cd /tmp/girocrab-main/GiroCrabApp && swift build -q && \
  .build/debug/GiroCrabMac --demo-seed --pick rewe --screenshot /tmp/vorher-standard.png)
# iPhone: im Worktree genauso mit xcodebuild aus /tmp/girocrab-main/GiroCrabApp/Xcode/ (siehe unten)
git worktree remove --force /tmp/girocrab-main   # erst nach macOS und iPhone
```

### macOS

Aus `GiroCrabApp/` nach `swift build -q` (`--screenshot` blendet den Demo-Hinweis aus):

```bash
APP=.build/debug/GiroCrabMac
$APP --demo-seed --pick rewe --screenshot /tmp/audit-standard.png
$APP --demo-seed --pick rewe --screenshot /tmp/audit-larger.png --text-size larger
$APP --demo-seed --pick rewe --screenshot /tmp/audit-dark.png --dark
$APP --demo-seed --pick rewe --screenshot /tmp/audit-redact.png --redact
$APP --demo-seed --pick rewe --screenshot /tmp/audit-narrow.png --window-width 900
$APP --demo-seed --pick rewe --screenshot /tmp/audit-tall.png --window-height 1900   # ganze Spalte
```

### iPhone

Auf dem iPhone schreibt `--screenshot` kein Bild, blendet aber wie auf dem Mac den Demo-Hinweis aus; aufgenommen
wird mit `simctl`. Mitteilungen einmal pro Simulator abschalten. `--pick` öffnet auf dem iPhone auch das
Buchungs-Sheet, das sonst nur durch Antippen erscheint. Aus `GiroCrabApp/Xcode/`, nach `./build-xcframework.sh`
(iOS-Teil):

```bash
UDID=$(xcrun simctl list devices booted | grep -Eo '[0-9A-F-]{36}' | head -1)   # oder Simulator starten
xcodebuild -quiet -project GiroCrab.xcodeproj -scheme GiroCrab -destination "id=$UDID" \
  -derivedDataPath /tmp/girocrab-dd CODE_SIGNING_ALLOWED=NO build
xcrun simctl install "$UDID" /tmp/girocrab-dd/Build/Products/Debug-iphonesimulator/GiroCrab.app
xcrun simctl spawn "$UDID" defaults write de.girocrab.app announcements.enabled -bool NO   # keine Mitteilungs-Banner

shot() {  # shot <Datei> [Argumente …]
  xcrun simctl terminate "$UDID" de.girocrab.app >/dev/null 2>&1 || true
  SIMCTL_CHILD_GIROCRAB_DEMO_SEED=1 xcrun simctl launch "$UDID" de.girocrab.app --demo-seed \
    --screenshot /dev/null "${@:2}" >/dev/null   # --screenshot: ohne Demo-Hinweis
  sleep 10   # Start mit Demo-Daten dauert; ein zu früher Screenshot zeigt den Startbildschirm
  xcrun simctl io "$UDID" screenshot "$1"
}
shot /tmp/ios-standard.png --pick rewe
shot /tmp/ios-redact.png --pick rewe --redact
xcrun simctl ui "$UDID" appearance dark && shot /tmp/ios-dark.png --pick rewe; xcrun simctl ui "$UDID" appearance light
xcrun simctl ui "$UDID" content_size accessibility-extra-large && shot /tmp/ios-larger.png --pick rewe
xcrun simctl ui "$UDID" content_size large
```

## Vorlage für den Bericht

```markdown
# UI-Audit: <Bereich>

- **Datum / Stand:** <Datum>, `main` @ <Commit>
- **Ansicht:** <was genau, auf welchen Plattformen>
- **Dateien:** `GiroCrabApp/Sources/GiroCrabUI/<Datei>.swift`
- **Werkzeug:** Impeccable `critique` (<n>/<max>) und `audit` (<n>/20), dazu GiroCrab-Regeln von Hand
- **Nachweis:** macOS (Standard, „Sehr groß“, dunkel, schmal, `--redact`), iPhone (Simulator, normal, `--redact`)

| ID | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|
| XYZ-1 | Barrierefreiheit | macOS | … | Fix | behoben |
| XYZ-2 | Konsistenz | beide | … | Issue | #… |
| XYZ-3 | Konsistenz | beide | … | Entscheidung | im RFC |

## XYZ-1: <kurzer Titel>
**Ort:** `<Datei>.swift:<Zeile>` · **Beobachtung:** … · **Vorschlag:** … · **Prüfung:** woran man die Verbesserung erkennt.
```
