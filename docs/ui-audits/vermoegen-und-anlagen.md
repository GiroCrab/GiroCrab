# UI-Audit: Vermögen und Anlagen

- **Ansicht:** Vermögensübersicht, Sparpläne & Perks, Einzeldepot und Sachwerte-Konto mit gemeinsamen Prüffällen für Sparplanraster, Wert und Zusammensetzung.
- **Dateien:** `InvestmentDashboardView.swift`, `BookingListView.swift`, `DepotDetailView.swift`, `DepotPositionsTable.swift`, `DepotValueChart.swift`, `AssetAccountFrame.swift`, `TangibleAccountDetailView.swift`, `TangibleAssetsUI.swift`, `WealthView.swift`, `AssetCompositionCard.swift`, `CompositionCharts.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` anhand von SwiftUI-Code und Demo-Aufnahmen, dazu GiroCrab-Checkliste.
- **Nachweis:** macOS 27.2.1, iPhone 18 Pro und Pro Max sowie iPad (iOS 27.2), Deutsch. Standard, große Schrift und Schwärzung mit Demo-Daten. [Prüfbuild vom 25.11.2026](README.md#prüfbuilds).

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

- **Nachprüfung:** 25.11.2026, `main` @ `09f11987`, VER-1 bis VER-6. [Prüfbuild](README.md#prüfbuilds).
- **Umgebung:** macOS 27.2.1, iPhone 18 Pro und iPad mini (iOS 27.2, Hochformat), Deutsch. Hell/Dunkel, große Schrift und `--redact=labels` nach [Prüfmatrix](../ui-audit.md#prüfmatrix).
- **Demo-Einstieg:** `--demo-seed --view vermoegen`, Reiter „Alles“, Inhaber „Alle“.
- **VER-1 bis VER-4:** Auf dem iPhone stapelt die Steuerung bei „Accessibility XL“ und die Summenzeilen bleiben ganz. Ohne Schuldenabzug heißt die Summe auf Mac und iPhone „Vermögenswerte“. Mit `--demo-filter wealth:cash` bleiben auf dem Mac die Legendeneinträge lesbar. Die geschwärzten Summen und Inhabernamen sind verdeckt.
- **Ringdiagramm:** Bei „Accessibility XL“ in „Zusammensetzung“ mit „Nach Konto“ zum Ring scrollen. Die iPhone-Aufnahme nutzte den lokalen Scroll-Patch `--audit-scroll-composition`. Zum Nachstellen genügt manuelles Scrollen. Dieselben Komponenten auch in Depot und Sachwerte-Konto prüfen.

- **Status:** Die vier ursprünglichen Befunde wurden über [#1861](https://github.com/GiroCrab/GiroCrab/pull/1861) behoben. [#998](https://github.com/GiroCrab/GiroCrab/issues/998) betrifft einen Datenfall, der sich mit dem Demo-Tresor nicht nachstellen ließ.
- **Früherer Bericht:** Code-Stellen, Lösungsvorschläge und Impeccable-Einzelbewertungen zu VER-1 bis VER-4. [Ausführlicher Vermögensbericht](https://github.com/GiroCrab/GiroCrab/blob/ba18ff6b47f2a7044cb1a12c16e12d49c94d2453/docs/ui-audits/vermoegen.md)

#### VER-5 · Ringsumme überdeckt Segmente

- **Ort:** `CompositionCharts.swift`, `CompositionRing`.
- **Beobachtung:** Bei „Accessibility XL“ wächst der Betrag weiter, obwohl der Durchmesser begrenzt bleibt. Auf iPhone und iPad ragt die Ringsumme in die farbigen Segmente.
- **Vorschlag:** Die Summe bei zu kleinem Innenradius außerhalb des Rings platzieren.
- **Prüfung:** Den beschriebenen Ringfall auf iPhone und iPad wiederholen. Summe und Segmente überlagern sich nicht. [Bildbelege (ZIP)][bildbelege] · `iphone-vermoegen-ring-gross.png`.

#### VER-6 · Legendenbeträge brechen innerhalb der Zahl um

- **Ort:** `CompositionCharts.swift`, `CompositionLegend`.
- **Beobachtung:** Auf dem iPhone teilen sich Label, Betrag und Anteil eine Zeile. Bei „Accessibility XL“ bricht etwa „29.798,1“ vor „6 €“ um. Auf dem iPad bleibt der Betrag ganz.
- **Vorschlag:** Bei geringer Breite Betrag und Anteil unter das Label setzen und den formatierten Betrag zusammenhalten.
- **Prüfung:** Den beschriebenen Ringfall am iPhone nachstellen. Jeder Legendeneintrag zeigt den Betrag samt Währung zusammenhängend. [Bildbelege (ZIP)][bildbelege] · `iphone-vermoegen-ring-gross.png`.

### Sparpläne & Perks

- **Demo-Einstieg:** `--demo-seed --view sparplaene`. „Monatliche Sparpläne“ auf dem Mac schmal mit „Sehr groß“ und auf dem iPhone mit „Accessibility XL“ vergleichen.

- **Offene Variante:** Saveback und Round up waren in diesem Demo-Stand nicht sichtbar. Die bedingte Perks-Karte braucht einen eigenen Demo-Fall.

#### SPA-1 · Sparplanraster verliert die Zuordnung

- **Ort:** `InvestmentDashboardView.swift`, `PlanGrid` mit 28 bzw. 40 pt breiten Monatsspalten.
- **Beobachtung:** Am schmalen Mac mit großer Schrift bleiben von den Planzeilen nur Signets übrig. Am iPhone bei „Accessibility XL“ überlappen sich Monatsüberschriften.
- **Vorschlag:** Die Mindestbreiten an Schriftgröße und Inhalt koppeln. Bei Platzmangel horizontal scrollen oder eine gestapelte Darstellung anbieten.
- **Prüfung:** „Monatliche Sparpläne“ auf Mac und iPhone prüfen. Namen, Monate und Beträge sind einander zuordenbar. Den Depot-Reiter „Sparpläne“ mitprüfen. [Bildbelege (ZIP)][bildbelege] · `mac-sparplaene-gross.png`, `iphone-gross-sparplaene.png`.

### Einzeldepot

- **Demo-Einstieg:** `--demo-seed --select "Welt-Depot"`. „Positionen“ auf dem iPhone bei Standardgröße und „Accessibility XL“ vergleichen. Auf dem Mac auch mit `--depot-tab activities` und `--depot-tab plans` starten.

- **Gemeinsame Komponenten:** Die iPad-Aufnahme mit `--redact=labels` verdeckt Positionsnamen, Werte und Diagramm. Für große Schrift sind die [Vermögensbefunde zur Zusammensetzung](#vermögen) und [SPA-1 zum Sparplanraster](#sparpläne--perks) relevant.

#### DEP-1 · Depotkopf und Wertverlauf laufen aus dem Bildschirm

- **Ort:** `AssetAccountFrame.swift` (`AssetHeaderCard`), `DepotDetailView.swift` und `DepotValueChart.swift`.
- **Beobachtung:** Bei `--select "Welt-Depot"` bleiben auf dem iPhone bei „Accessibility XL“ Wert und Kopf nebeneinander. Depotwert, Betragsachse und Dokumentaktion enden am Bildschirmrand.
- **Vorschlag:** Den gemeinsamen Kopf bei Platzmangel stapeln. Verlauf und Dokumentaktion separat auf überbreite Inhalte prüfen.
- **Prüfung:** Im Depot-Reiter „Positionen“ bei „Accessibility XL“ Wert, Achse und Dokumentaktion vollständig sehen und die Aktion öffnen. Mit [SAC-1](#sachwerte-konto) abgleichen. [Bildbelege (ZIP)][bildbelege] · `iphone-gross-depot.png`.

### Sachwerte-Konto

- **Demo-Einstieg:** `--demo-seed --select Sachwerte`. Bei Standardgröße und „Accessibility XL“ den Kopf „Sachwerte · Gesamt“ vergleichen. Eine Einzelposition auf dem Mac mit `--pick Einfamilienhaus` anwählen.

- **Schwärzung:** Die iPad-Aufnahme mit `--redact=labels` verdeckt Namen, Mengen und Beträge. Die offenen [Vermögensbefunde](#vermögen) zur Diagrammkomponente gelten als weitere Prüffälle für dieses Konto.

#### SAC-1 · Sachwertekopf wird abgeschnitten

- **Ort:** `AssetAccountFrame.swift` (`AssetHeaderCard`) und `TangibleAccountDetailView.swift`.
- **Beobachtung:** Auf dem iPhone bei „Accessibility XL“ läuft der Wert „522.187,19 €“ im Kopf „Sachwerte · Gesamt“ nach rechts aus dem Bild. Der Bewertungstext bleibt nur in Bruchstücken sichtbar.
- **Vorschlag:** Den gemeinsamen Kopf wie beim Depot stapeln und dem Bewertungstext seine volle Breite geben.
- **Prüfung:** Gesamtansicht mit großem Text prüfen. Wert und Bewertung sind vollständig lesbar, die Dokumentaktion bleibt erreichbar. Mit [DEP-1](#einzeldepot) abgleichen. [Bildbelege (ZIP)][bildbelege] · `iphone-gross-sachwerte.png`.

[bildbelege]: https://github.com/user-attachments/files/35449133/PR-Bildbelege.zip
