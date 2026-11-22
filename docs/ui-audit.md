# UI-Audits: kleine Prüfungen einzelner Ansichten

Ein UI-Audit prüft **eine** Ansicht nach einem kurzen Raster und macht daraus wenige, gut abgrenzbare
Aufgaben. Ziel ist nicht, die ganze App auf einmal zu bewerten, sondern Schritt für Schritt
nachvollziehbare Verbesserungen zu sammeln. Hintergrund und Diskussion: RFC #1673.

Maßstab sind [`PRODUCT.md`](../PRODUCT.md) (wofür und für wen) und [`DESIGN.md`](../DESIGN.md)
(wie es heute aussieht). Technische Regeln stehen in [`CLAUDE.md`](../CLAUDE.md).

## Ablauf

1. **Ansicht wählen.** Ein Bereich pro Audit, z. B. „Buchungs-Inspektor“ oder „Seitenleiste“.
   Vorher in den offenen Issues mit `[UI-Audit]` nachsehen, ob schon jemand daran arbeitet.
2. **Screenshots mit Demo-Daten** (`--demo-seed` bzw. `GIROCRAB_DEMO_SEED=1`, nie mit echten Daten), siehe
   [Screenshots](#screenshots).
3. **Prüfen** nach dem [Raster](#raster), von Hand oder mit einem Agenten ([Impeccable](#mit-impeccable)).
   Jede Beobachtung mit Ort (Datei, View), Plattform und Kriterium notieren.
4. **Bericht schreiben** in `docs/ui-audits/<bereich>.md` (Vorlage unten). Jede Beobachtung bekommt eine
   ID aus Kürzel und Nummer, z. B. `INS-1`.
5. **Sortieren.** Jedes Finding bekommt genau einen Weg:
   - **Fix:** klein und unstrittig → direkt im Audit-PR beheben, ein Commit pro Finding.
   - **Issue:** klar, aber eigene Arbeit → höchstens fünf Issues pro Audit, Titel
     `[UI-Audit] <Bereich>: <Beobachtung>`, ggf. `good first issue` / `help wanted`.
   - **Entscheidung:** betrifft mehrere Ansichten (gemeinsame Abstände, Radien, Betragsgrößen) →
     bleibt im Bericht und wird im RFC-Issue zur Diskussion gestellt, nicht nebenbei geändert.
   - **Verworfen:** nach Prüfung kein Problem → mit Begründung im Bericht lassen.
6. **Nachvollziehbar halten.** Commit-Nachrichten nennen die ID (`fix(ui): … (INS-1)`), der Bericht nennt
   für jedes Finding den Status und den Commit bzw. das Issue. Eine kurze Zusammenfassung kommt als
   Kommentar ins RFC-Issue.

## Raster

| Kriterium | Frage |
|---|---|
| Lesbarkeit und Hierarchie | Sind wichtige Informationen schnell erkennbar? Stehen vergleichbare Beträge auf einer Linie und sind sie einheitlich formatiert? Haben Beträge, Prozentwerte und Daten gleich breite Ziffern (`monospacedDigit`)? |
| Dichte und Konsistenz | Passen Abstände, Radien und Symbole zu benachbarten Ansichten? Besitzen interaktive Elemente auf mobilen Plattformen ausreichende Trefferflächen (iOS ≥ 44 pt, Android ≥ 48 dp)? |
| Texte und Zustände | Ist bei leeren Ansichten oder Fehlern klar, was passiert ist und was man tun kann? Bleiben lange deutsche Texte bei schmalem Fenster und auf dem iPhone lesbar? |
| Hell und dunkel | Tragen die `Theme.*`-Farben in beiden Modi dieselbe Bedeutung? Bleibt der Kontrast in beiden Modi ausreichend? |
| Barrierefreiheit | Bleibt die Ansicht bei größerer Schrift nutzbar? Stimmen Kontrast und VoiceOver-Beschriftungen? Lassen sich Bedienelemente am Desktop per Tastatur erreichen und ist der Fokus sichtbar? Läuft jede Animation über `Motion` („Bewegung reduzieren“)? |
| Datenschutz | Werden sensible Angaben in Screenshots für „Problem melden“ geschwärzt, auch in Titeln? |

Schnelle Code-Hinweise für die Kriterien (Treffer sind Kandidaten, keine Befunde):

```bash
cd GiroCrabApp/Sources/GiroCrabUI
grep -n '\.font(\.system(' <Datei>.swift          # Text, der der Schriftgröße nicht folgt
grep -n 'frame(width:\|frame(height:\|minHeight:' <Datei>.swift   # feste Maße: wachsen sie mit textScale?
grep -n 'navigationTitle\|\.help(' <Datei>.swift  # Parametertexte: sensible Inhalte ersetzt?
grep -n 'cornerRadius:' <Datei>.swift             # Radien im Vergleich zu DESIGN.md › Shapes
grep -n 'withAnimation(\|\.animation(' <Datei>.swift | grep -v Motion   # Animation ohne „Bewegung reduzieren“
grep -n 'Color(\|\.foregroundStyle(\.' <Datei>.swift   # Farben außerhalb von Theme.*
```

## Screenshots

Immer mit Demo-Daten. Aus `GiroCrabApp/` nach `swift build -q`:

```bash
APP=.build/debug/GiroCrabMac
$APP --demo-seed --pick rewe --screenshot /tmp/audit-standard.png
$APP --demo-seed --pick rewe --screenshot /tmp/audit-larger.png --text-size larger
$APP --demo-seed --pick rewe --screenshot /tmp/audit-dark.png --dark
$APP --demo-seed --pick rewe --screenshot /tmp/audit-redact.png --redact
$APP --demo-seed --pick rewe --screenshot /tmp/audit-narrow.png --window-width 900
```

`--pick <Text>` wählt die erste Buchung, deren Händler oder Gegenpartei den Text enthält; `--select <Konto>`
wählt ein Konto. `--screenshot` gibt es nur auf macOS. iOS: App im gestarteten Simulator mit Demo-Daten
starten und das Bild mit `simctl` aufnehmen (wie `scripts/verify_ui.sh`):

```bash
xcrun simctl terminate booted de.girocrab.app || true
SIMCTL_CHILD_GIROCRAB_DEMO_SEED=1 xcrun simctl launch booted de.girocrab.app --pick rewe
sleep 1.5
xcrun simctl io booted screenshot /tmp/audit-ios.png
```

Screenshots gehören in den PR oder das Issue, nicht ins Repository.

## Mit Impeccable

[Impeccable](https://impeccable.style/docs/) ist ein Agent-Skill für Design-Reviews (Installation laut
Projektseite, lokal; der Skill wird nicht ins Repository eingebettet). Er liest `PRODUCT.md` und `DESIGN.md`
aus dem Projektordner.

- `critique <Ansicht>`: Design-Bewertung (Hierarchie, Dichte, Konsistenz, Texte).
- `audit <Ansicht>`: technische Prüfung; für SwiftUI ein Code-Audit mit den iOS-Leitlinien.
- Der automatische Detector liest nur Web-Code; für die nativen Apps gibt er kein Ergebnis aus.

Die Ausgabe des Agenten ist ein Vorschlag. Jede Beobachtung wird vor dem Bericht im Code oder im Screenshot
bestätigt; was sich nicht bestätigen lässt, kommt nicht hinein. Der Agent ändert im Audit keinen Code.

## Vorlage für den Bericht

```markdown
# UI-Audit: <Bereich>

- **Datum / Stand:** <Datum>, `main` @ <Commit>
- **Plattformen:** macOS, iOS
- **Dateien:** `GiroCrabApp/Sources/GiroCrabUI/<Datei>.swift`
- **Werkzeug:** Raster von Hand / Impeccable `critique` + `audit`

| ID | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|
| XYZ-1 | Barrierefreiheit | macOS | … | Fix | behoben in <Commit> |
| XYZ-2 | Konsistenz | beide | … | Issue | #… |
| XYZ-3 | Konsistenz | beide | … | Entscheidung | im RFC |

## XYZ-1: <kurzer Titel>
**Ort:** `<Datei>.swift:<Zeile>` · **Beobachtung:** … · **Vorschlag:** … · **Prüfung:** woran man die Verbesserung erkennt.
```
