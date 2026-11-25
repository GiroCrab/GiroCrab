# UI-Audit: Anlagen

- **Ansicht:** Vermögensübersicht, Sparpläne & Perks, Einzeldepot und Sachwerte-Konto mit gemeinsamen Komponenten für Sparplanraster, Wert und Zusammensetzung.
- **Dateien:** `InvestmentDashboardView.swift`, `BookingListView.swift`, `DepotDetailView.swift`, `DepotPositionsTable.swift`, `DepotValueChart.swift`, `AssetAccountFrame.swift`, `TangibleAccountDetailView.swift`, `TangibleAssetsUI.swift`, `WealthView.swift`, `AssetCompositionCard.swift`, `CompositionCharts.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` anhand von SwiftUI-Code und Demo-Aufnahmen, dazu GiroCrab-Checkliste.
- **Nachweis:** macOS 27.2.1, iPhone 18 Pro und Pro Max sowie iPad (iOS 27.2), Deutsch. Standard, große Schrift und Schwärzung mit Demo-Daten. iOS-Prüfkopie mit [#1919](https://github.com/GiroCrab/GiroCrab/pull/1919).

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| VER-1 | P1 | Adaptivität | iPhone | Die Diagrammsteuerung verbreiterte bei „Accessibility XL“ die gesamte Ansicht. Sie stapelt sich jetzt bei Bedarf | Fix | `fixed` |
| VER-2 | P1 | Lesbarkeit | iPhone | Die Summenzeilen brachen mitten im Betrag um. Beträge erhalten bei Bedarf eine eigene Zeile | Fix | `fixed` |
| VER-3 | P1 | Fachliche Beschriftung | macOS, iPhone | Bei ausgeschaltetem Schuldenabzug stand „Nettovermögen“. Die Beschriftung folgt jetzt den Schaltern | Fix | `fixed` |
| VER-4 | P2 | Kontrast | macOS | Ein aktiver Diagrammfilter blendete auch die Texte weiterhin auswählbarer Einträge ab | Fix | `fixed` |
| VER-5 | P2 | Lesbarkeit | iPhone, iPad | Bei „Accessibility XL“ überlappt die Summe die farbigen Segmente des Ringdiagramms | Issue | `open` |
| VER-6 | P2 | Lesbarkeit | iPhone | Bei „Accessibility XL“ brechen Beträge in der Ringlegende mitten in der Zahl um | Issue | `open` |
| SPA-1 | P1 | Lesbarkeit | macOS, iPhone | Bei großer Schrift geht im Sparplan-Raster die Zuordnung verloren. Auf dem Mac bleiben von beiden Plänen nur die Signets sichtbar. Auf dem iPhone überlappen die Monatsüberschriften | Issue | `open` |
| DEP-1 | P1 | Adaptivität | iPhone | Bei „Accessibility XL“ steht der Depotwert rechts teilweise außerhalb des Bildschirms. Auch Betragsachse und Dokumentaktion laufen rechts aus der Ansicht | Issue | `open` |
| SAC-1 | P1 | Adaptivität | iPhone | Bei „Accessibility XL“ ragen „Wert heute“, Betragsachse und Dokumentaktion rechts aus der sichtbaren Ansicht. Der Bewertungstext links ist nur in Bruchstücken sichtbar | Issue | `open` |

## Impeccable-Bewertung

| Ansicht | Datum / Stand | Critique | Technisches Audit | Kurzbewertung |
|---|---|---|---|---|
| Vermögen | 24.11.2026, `main` @ `91988771` | 24/36 | 13/20 | Die Ausgangsbewertung beruht auf den damaligen Problemen mit Summenbeschriftung, Kontrast und dem Layout bei großer Schrift. |
| Sparpläne & Perks | 25.11.2026, `main` @ `09f11987` | 25/36 | 13/20 | Der Vergleich von Plan und Monatsbetrag funktioniert bei Standardgröße. Bei großer Schrift lässt das Raster die Zuordnung von Betrag und Ausführung zum Plan schwer erkennen. Der geprüfte Ausschnitt hatte keinen eigenen Fehlerfall. |
| Einzeldepot | 25.11.2026, `main` @ `09f11987` | 26/36 | 13/20 | Die Trennung von Bestand, Aktivitäten und Sparplänen unterstützt die Arbeit im Depot. Bei großer Schrift gehen Informationen bereits im Kopf des Reiters „Positionen“ verloren. |
| Sachwerte-Konto | 25.11.2026, `main` @ `09f11987` | 26/36 | 13/20 | Einstandswert, aktueller Wert und Zuordnung nach Anlageklasse sind bei Standardgröße schnell zu vergleichen. Bei großer Schrift wird bereits der Kopf zur Hürde. Der Goldpreisabruf war außerhalb dieses Durchgangs. |

## Prüfnachweis

### Vermögen

- **Nachprüfung:** 25.11.2026, `main` @ `09f11987`, VER-1 bis VER-6. Für iOS lag der Swift-Buildfix aus [#1919](https://github.com/GiroCrab/GiroCrab/pull/1919) in einer temporären Kopie.
- **Umgebung:** macOS 27.2.1, iPhone 18 Pro und iPad mini (iOS 27.2, Hochformat), Deutsch. Hell/Dunkel, große Schrift und `--redact=labels` nach [Prüfmatrix](../ui-audit.md#prüfmatrix).
- **Demo-Einstieg:** `--demo-seed --view vermoegen`, Reiter „Alles“, Inhaber „Alle“.
- **VER-1 bis VER-4:** Auf dem iPhone stapelt die Steuerung bei „Accessibility XL“ und die Summenzeilen bleiben ganz. Ohne Schuldenabzug heißt die Summe auf Mac und iPhone „Vermögenswerte“. Mit `--demo-filter wealth:cash` bleiben auf dem Mac die Legendeneinträge lesbar. Die geschwärzten Summen und Inhabernamen sind verdeckt.
- **VER-5 und VER-6:** Auf dem iPhone in „Zusammensetzung“ mit „Nach Konto“ zum Ring scrollen. Die Zahl in der Ringmitte ragt über das Loch hinaus. In der Legende trennt sich z. B. `29.798,1` von `6 €` (`iphone-vermoegen-ring-gross.png`). Auf dem iPad überlappt die Ringsumme ebenfalls, während die Beträge der Legende ganz bleiben. Für die Aufnahme auf dem iPhone sprang die Prüfkopie per Startargument zur Karte.
- **Code und Abnahme:** `CompositionRing` begrenzt den Durchmesser, der Betrag skaliert weiter. In `CompositionLegend` teilen sich Label, Betrag und Anteil eine Zeile. Bei großer Schrift müssen die Ringsumme ohne Überlagerung und jeder Legendenbetrag zusammenhängend lesbar sein. Die gemeinsame Komponente auch in Depot und Sachwerte-Konto prüfen.
- **Status:** Die vier ursprünglichen Befunde wurden über [#1861](https://github.com/GiroCrab/GiroCrab/pull/1861) behoben. [#998](https://github.com/GiroCrab/GiroCrab/issues/998) betrifft einen Datenfall, der sich mit dem Demo-Tresor nicht nachstellen ließ.

### Sparpläne & Perks

- **Demo-Einstieg:** `--demo-seed --view sparplaene`. „Monatliche Sparpläne“ auf dem Mac schmal mit „Sehr groß“ und auf dem iPhone mit „Accessibility XL“ vergleichen.
- **SPA-1:** `PlanGrid` reserviert pro Monat 28 bzw. 40 pt, während die Texte mitwachsen. In `mac-sparplaene-gross.png` verschwinden die Namen „iShares Core MSCI World“ und „Vanguard FTSE All-World“. In `iphone-gross-sparplaene.png` überschneiden sich die Monate. Abnahme mit eindeutig zuordenbaren Namen, Monatsmarkern und Beträgen. Dasselbe Raster im Depot-Reiter „Sparpläne“ prüfen.
- **Offene Variante:** Saveback und Round up waren in diesem Demo-Stand nicht sichtbar. Die bedingte Perks-Karte braucht einen eigenen Demo-Fall.

### Einzeldepot

- **Demo-Einstieg:** `--demo-seed --select "Welt-Depot"`. „Positionen“ auf dem iPhone bei Standardgröße und „Accessibility XL“ vergleichen. Auf dem Mac auch mit `--depot-tab activities` und `--depot-tab plans` starten.
- **DEP-1:** In `iphone-gross-depot.png` bleibt `AssetHeaderCard` nebeneinander angeordnet und der Wert „31.380,00 €“ endet am Bildschirmrand. Der gemeinsame Rahmen enthält auch den Wertverlauf und die Dokumentaktion. Mit dem [Sachwerte-Befund SAC-1](#sachwerte-konto) gemeinsam nachprüfen, dass Titel, Wert und Aktionen in den Bildschirm passen.
- **Gemeinsame Komponenten:** Die iPad-Aufnahme mit `--redact=labels` verdeckt Positionsnamen, Werte und Diagramm. Für große Schrift sind die [Vermögensbefunde zur Zusammensetzung](#vermögen) und [SPA-1 zum Sparplanraster](#sparpläne--perks) relevant.

### Sachwerte-Konto

- **Demo-Einstieg:** `--demo-seed --select Sachwerte`. Bei Standardgröße und „Accessibility XL“ den Kopf „Sachwerte · Gesamt“ vergleichen. Eine Einzelposition auf dem Mac mit `--pick Einfamilienhaus` anwählen.
- **SAC-1:** `AssetHeaderCard` zeigt Namen und aktuellen Wert in `iphone-gross-sachwerte.png` nebeneinander. Nach dem Fix soll „522.187,19 €“ vollständig sichtbar sein und der Bewertungstext verständlich bleiben. Wegen des gemeinsamen Rahmens zusammen mit [DEP-1](#einzeldepot) nachprüfen.
- **Schwärzung:** Die iPad-Aufnahme mit `--redact=labels` verdeckt Namen, Mengen und Beträge. Die offenen [Vermögensbefunde](#vermögen) zur Diagrammkomponente gelten als weitere Prüffälle für dieses Konto.
