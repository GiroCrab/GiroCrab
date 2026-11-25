# UI-Audit: Vermögen

- **Stand:** 24.11.2026, `main` @ `91988771`
- **Ansicht:** SwiftUI-Vermögensübersicht (`WealthView.swift`) mit Inhaberfilter, Summenkarte und Zusammensetzung. Die Depot-Einzelansicht gehört als Nutzerin gemeinsamer Komponenten zur späteren Prüfung dieser Komponenten.
- **Plattformen und Nachweis:** macOS in heller und dunkler Darstellung, schmal und mit „Sehr groß“. iPhone 18 Pro und iPad mini im iOS-27.2-Simulator. Auf dem iPhone zusätzlich „Accessibility XL“ und `--redact=labels`. Alle Aufnahmen stammen aus dem Demo-Tresor.
- **Methode:** Impeccable `critique` und `audit` für native Ansichten. Die Bewertungen beruhen auf SwiftUI-Code und Demo-Screenshots. Produktregeln für Schwärzung, Währungen und Beträge wurden zusätzlich nach [Audit-Anleitung](../ui-audit.md) geprüft.

Die Gruppen nach Kontoart und Liquidität sind bereits in `main`. Issue #998 betrifft konkrete Daten aus einem realen Tresor. Die Demo-Prüfung liefert dafür keine Reproduktion. [PR #1638](https://github.com/GiroCrab/GiroCrab/pull/1638) · [PR #1704](https://github.com/GiroCrab/GiroCrab/pull/1704) · [Issue #998](https://github.com/GiroCrab/GiroCrab/issues/998)

| ID | Priorität | Plattform | Beobachtung | Änderung |
|---|---|---|---|---|
| VER-1 | P1 | iPhone | Bei „Accessibility XL“ und ausgeklappter Zusammensetzung ragt die Vermögensansicht links und rechts aus dem sichtbaren Bereich. | Die gemeinsame Diagrammsteuerung stapelt die Bedienelemente bei knapper Breite. |
| VER-2 | P1 | iPhone | Die Summenzeilen für Vermögenswerte und Verbindlichkeiten trennen Zahlen mitten im Betrag. | Beträge bleiben zusammen und erhalten bei Bedarf eine eigene Zeile. |
| VER-3 | P1 | macOS, iPhone | Bei ausgeschaltetem Schuldenabzug steht die Bruttosumme unter „Nettovermögen“. | Die Summenbeschriftung und der Hilfetext zur Inhaberauswahl folgen dem Schuldenabzug. |
| VER-4 | P2 | macOS | Weiterhin auswählbare Legendeneinträge verblassen bei aktivem Filter so weit, dass der Textkontrast abfällt. | Ringlegende und Rangliste dimmen Farbpunkte oder Balken. Text und Beträge behalten ihre Deckkraft. |

## VER-1 · Zusammensetzung verbreitert die Ansicht

**Befund.** Im Reiter „Alles“ fehlen bei „Accessibility XL“ der linke Rand des Titels und der rechte Rand von Summe und Schaltern. Der Überlauf ist auch in „Cash“, „Wertpapiere“ und „Sachwerte“ sichtbar. Klappt man „Zusammensetzung“ in „Alles“ ein, passen Kopf und Summenkarte wieder in die Gerätebreite. Der unabhängige Umbruch der Summenzeilen bleibt sichtbar (VER-2).

**Code und Prüfung.** `WealthView.body` baut die ganze Ansicht in einem vertikalen `ScrollView`. `SliceCompositionCard.controls` setzte Gruppierung, Darstellung und Teilen unter iOS in eine horizontale Reihe. In einer separaten Prüfkopie brachte eine gestapelte Anordnung „Alles“ und „Cash“ wieder innerhalb der Bildschirmbreite. Jetzt wählt `ViewThatFits` die Anordnung passend zur verfügbaren Breite. Die gemeinsame Komponente wird auch in Depot- und Sachwerte-Konten verwendet. Belege `girocrab-vermoegen-iphone-gross.png`, `girocrab-vermoegen-iphone-gross-ohne-zusammensetzung.png` und `girocrab-vermoegen-probe-vermoegen-cash.png`.

## VER-2 · Beträge in den Summenzeilen brechen um

**Befund.** Bei eingeklappter Zusammensetzung steht auf dem iPhone `606.224,` in einer Zeile und `75 €` darunter. Auch die Verbindlichkeiten werden mitten in der Zahl umgebrochen. Das lässt sich getrennt von VER-1 nachstellen.

**Code und Prüfung.** In `WealthView.bar` stehen Beschriftung und Betrag nebeneinander, wenn beide Platz haben. Andernfalls erhält der Betrag eine eigene Zeile. Überbreite Beträge sind innerhalb dieser Zeile scrollbar. Ziffern bleiben gleichbreit und Beträge werden beim Schwärzen abgedeckt. Beleg `girocrab-vermoegen-iphone-gross-ohne-zusammensetzung.png`.

## VER-3 · Bruttosumme trägt die Beschriftung „Nettovermögen“

**Befund.** Mit Demo-Daten sind es `414.214,84 €` nach Abzug von `192.009,91 €` Verbindlichkeiten. Wird „Kredite und Karten abziehen“ ausgeschaltet, steigt der Wert auf `606.224,75 €`. Die Beschriftung bleibt „Nettovermögen“.

**Code und Prüfung.** `WealthView` zeigt bei aktivem Abzug „Nettovermögen“ und bei ausgeschaltetem Abzug „Vermögenswerte“. Beide Bezeichnungen berücksichtigen auch ausgeblendete Immobilien. Die Tooltips in der Inhaberauswahl folgen den globalen Schaltern und sind für alle Inhaber gleich. Qt verwendet dieselben Begriffe. Belege `girocrab-vermoegen-mac-ohne-schulden.png` und `girocrab-vermoegen-iphone-ohne-schulden.png`.

## VER-4 · Ein aktiver Filter senkt den Legendenkontrast

**Befund.** Nach Auswahl von „Cash“ werden „Wertpapiere“ und „Sachwerte“ samt Beträgen stark abgeschwächt. Die Zeilen bleiben als nächster Filter auswählbar.

**Code und Prüfung.** `CompositionLegend` setzte die Deckkraft solcher Zeilen auf `0,35`. Die Mischung von `Theme.secondary` und Kartenfläche ergab für sekundären Text modelliert etwa `1,7:1` in Hell und `2,1:1` in Dunkel. GiroCrab verlangt `4,5:1` für Text. Jetzt dimmen nur die Farbpunkte der Ringlegende und die Balken der Rangliste. Bezeichnung, Beträge und Prozente behalten die vollen Textfarben, die Auswahl ist zusätzlich halbfett. Belege `girocrab-vermoegen-mac-filter.png` und `girocrab-vermoegen-mac-filter-dunkel.png`.

## Impeccable-Bewertung

**Critique 24/36.** Auf dem Mac lassen sich Vermögensklassen und Inhaber zügig vergleichen. Der Diagrammfilter hat mit „Alle anzeigen“ einen sichtbaren Rückweg. Auf dem iPhone werden bei sehr großer Schrift die primären Werte schwer lesbar. Wer die Summe nach Umschalten des Schuldenabzugs vergleicht, muss sich ihre Bedeutung aus dem Schalter erschließen.

| Heuristik | Punkte | Beobachtung |
|---|---|---|
| Sichtbarkeit des Status | 3 | Ansichtsreiter, Inhaberauswahl und Diagrammfilter zeigen ihren Zustand. |
| Sprache und fachliches Modell | 2 | Die Beschriftung „Nettovermögen“ bleibt im Bruttomodus stehen. |
| Kontrolle und Rückweg | 3 | Filter lassen sich zurücknehmen, Zusammensetzung lässt sich einklappen. |
| Konsistenz und Standards | 3 | Depot und Vermögen teilen Gruppierung, Legende und Diagrammsteuerung. |
| Fehlervermeidung | 3 | Summen und Filter sind sichtbar getrennt. Die fehlende Bezeichnung im Bruttomodus erschwert den Vergleich. |
| Wiedererkennen | 3 | Beschriftete Reiter und sichtbare Auswahl helfen bei der Orientierung. |
| Flexibilität | 3 | Diagrammdarstellung, Gruppierung und Inhaber sind unabhängig wählbar. |
| Visuelle Hierarchie | 2 | Clipping und ausgeblendete Legendentexte erschweren den Zugriff auf Werte. |
| Erholung von Fehlern | n/a | Im geprüften Ablauf wurde keine Fehlermeldung ausgelöst. |
| Hilfe und Dokumentation | 2 | Ein Hinweis erklärt den Cash-Verlauf. Eine getrennte Erklärung der gezeigten Netto- und Bruttowerte fehlt. |

**Technisches Audit 13/20.** Die Ansicht folgt bei Standardgröße den nativen Navigations- und Bedienmustern. Der kritische Fall liegt bei sehr großer Schrift auf dem iPhone.

| Dimension | Punkte | Beobachtung |
|---|---|---|
| Barrierefreiheit | 2 | Summenumbruch und niedriger Kontrast auswählbarer Legendenzeilen. |
| Performance | 3 | Historie und Positionen verwenden `vaultOffMain`. Die Einschätzung beruht auf Code und Demo-Lauf, ohne Profiling-Messung. |
| Erscheinungsbild | 3 | Konsistente Theme-Tokens in Hell und Dunkel. Die zusätzliche Deckkraft im Filterzustand beeinträchtigt den Kontrast. |
| Plattformtreue | 3 | Native Navigation, Picker, Schalter und Teilen. Mac und iOS passen die Darstellung der Gruppierung an. |
| Adaptivität | 2 | Standardgröße auf iPhone und iPad und die schmale Mac-Ansicht funktionieren. „Accessibility XL“ überschreitet die iPhone-Breite. |

## GiroCrab-Regeln

- **Schwärzung:** Die Demo-Aufnahme im Cash-Reiter verdeckt persönliche Kontonamen, Beträge, Anteile und die Zeichnung. Allgemeine Reiter und Bedienelemente bleiben lesbar.
- **Währungen:** `WealthSummary` addiert nur aktive, in der Gesamtsumme enthaltene EUR-Konten. Die vorhandenen Vermögens- und Kontrasttests liefen auf Mac durch (21 Tests).
- **Schrift und Beträge:** Der Mac-Umbruch von Ring und Legende funktioniert bei 900 pt und „Sehr groß“. Die Unterbrechung von Zahlen bei „Accessibility XL“ steht in VER-2.
