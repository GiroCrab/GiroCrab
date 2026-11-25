# UI-Audit: Vermögen

- **Datum / Stand:** 24.11.2026, `main` @ `91988771`.
- **Ansicht:** Vermögensübersicht mit Inhaberfilter, Summenkarte und Zusammensetzung auf macOS, iPhone und iPad.
- **Dateien:** `WealthView.swift`, `AssetCompositionCard.swift`, `CompositionCharts.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` plus GiroCrab-Checkliste.
- **Nachweis:** Demo-Aufnahmen in Hell, Dunkel, bei großer Schrift und mit Schwärzung. [Fixes und Prüfnachweis in #1861](https://github.com/GiroCrab/GiroCrab/pull/1861)

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| VER-1 | P1 | Adaptivität | iPhone | Die Diagrammsteuerung verbreiterte bei „Accessibility XL“ die gesamte Ansicht. Sie stapelt sich jetzt bei Bedarf | Fix | `fixed` |
| VER-2 | P1 | Lesbarkeit | iPhone | Die Summenzeilen brachen mitten im Betrag um. Beträge erhalten bei Bedarf eine eigene Zeile | Fix | `fixed` |
| VER-3 | P1 | Fachliche Beschriftung | macOS, iPhone | Bei ausgeschaltetem Schuldenabzug stand „Nettovermögen“. Die Beschriftung folgt jetzt den Schaltern | Fix | `fixed` |
| VER-4 | P2 | Kontrast | macOS | Ein aktiver Diagrammfilter blendete auch die Texte weiterhin auswählbarer Einträge ab | Fix | `fixed` |
| VER-5 | P2 | Lesbarkeit | iPhone, iPad | Bei „Accessibility XL“ überlappt die Summe die farbigen Segmente des Ringdiagramms | Issue | `open` |
| VER-6 | P2 | Lesbarkeit | iPhone | Bei „Accessibility XL“ brechen Beträge in der Ringlegende mitten in der Zahl um | Issue | `open` |

Alle vier ursprünglichen Befunde wurden über [#1861](https://github.com/GiroCrab/GiroCrab/pull/1861) behoben.
[#998](https://github.com/GiroCrab/GiroCrab/issues/998) bleibt für einen Datenfall offen, der sich mit dem Demo-Tresor nicht nachstellen ließ.

## Impeccable-Bewertung

**Critique 24/36 · Technisches Audit 13/20.** Die Ausgangsbewertung beruht auf den damaligen Problemen mit
Summenbeschriftung, Kontrast und dem Layout bei großer Schrift.

## Prüfnachweis

- **Nachprüfung:** 25.11.2026, `main` @ `09f11987`, VER-1 bis VER-6. Für iOS lag der Swift-Buildfix aus [#1919](https://github.com/GiroCrab/GiroCrab/pull/1919) in einer temporären Kopie.
- **Umgebung:** macOS 27.2.1, iPhone 18 Pro und iPad mini (iOS 27.2, Hochformat), Deutsch. Hell/Dunkel, große Schrift und `--redact=labels` nach [Prüfmatrix](../ui-audit.md#prüfmatrix).
- **Demo-Einstieg:** `--demo-seed --view vermoegen`, Reiter „Alles“, Inhaber „Alle“.
- **VER-1 bis VER-4:** Auf dem iPhone stapelt die Steuerung bei „Accessibility XL“ und die Summenzeilen bleiben ganz. Ohne Schuldenabzug heißt die Summe auf Mac und iPhone „Vermögenswerte“. Mit `--demo-filter wealth:cash` bleiben auf dem Mac die Legendeneinträge lesbar. Die geschwärzten Summen und Inhabernamen sind verdeckt.
- **VER-5 und VER-6:** Auf dem iPhone in „Zusammensetzung“ mit „Nach Konto“ zum Ring scrollen. Die Zahl in der Ringmitte ragt über das Loch hinaus. In der Legende trennt sich z. B. `29.798,1` von `6 €` (`iphone-vermoegen-ring-gross.png`). Auf dem iPad überlappt die Ringsumme ebenfalls, während die Beträge der Legende ganz bleiben. Für die Aufnahme auf dem iPhone sprang die Prüfkopie per Startargument zur Karte.
- **Code und Abnahme:** `CompositionRing` begrenzt den Durchmesser, der Betrag skaliert weiter. In `CompositionLegend` teilen sich Label, Betrag und Anteil eine Zeile. Bei großer Schrift müssen die Ringsumme ohne Überlagerung und jeder Legendenbetrag zusammenhängend lesbar sein. Die gemeinsame Komponente auch in Depot und Sachwerte-Konto prüfen.
