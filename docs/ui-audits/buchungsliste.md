# UI-Audit: Buchungsliste

- **Datum / Stand:** 25.11.2026, `main` @ `09f11987`, einschließlich direkter Kategorieauswahl aus [#1890](https://github.com/GiroCrab/GiroCrab/pull/1890).
- **Ansicht:** Tagesgruppen, Buchungszeilen, Auswahl und Kategorie-Direktwahl. Kontoübersicht, Filterleiste und Inspektor haben eigene Audit-Bereiche.
- **Dateien:** `BookingListView.swift`, `BookingRowText.swift`, `BookingCategoryMenu.swift` und `TagsUI.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` anhand von SwiftUI-Code und Demo-Aufnahmen, dazu die GiroCrab-Checkliste.
- **Nachweis:** macOS 27.2.1, iPhone 18 Pro und iPad mini (iOS 27.2, Hochformat, Deutsch). Demo-Aufnahmen in Hell, Dunkel, bei großer Schrift und mit `--redact=labels`, auf iPad Hell bei Standardgröße und „Accessibility XL“.

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| BUC-1 | P2 | Lesbarkeit | iPhone, macOS | In der Standardliste wird „Supermarkt“ auf dem iPhone zu „S…“. Auf dem schmalen Mac mit großer Schrift bleibt „Super…“. Der Tag daneben beansprucht weiter Platz | Issue | `open` |
| BUC-2 | P2 | Auffindbarkeit | macOS | In der Kompaktliste fehlt bei „Bäckerei Konditorei König“ die Kategorie-Direktwahl, obwohl der Tag angezeigt wird | Issue | `open` |
| BUC-3 | P2 | Kontrast | macOS | „Supermarkt“ erreicht auf dem blauen Hintergrund der ausgewählten REWE-Zeile in Hell etwa 3,9:1 | Issue | `open` |

## Impeccable-Bewertung

**Critique 25/36 · Technisches Audit 12/20.** Tagesgruppen, gleichbreite Beträge und Auswahlmarkierung unterstützen das Scannen. Beim monatlichen Kategorisieren am Mac erschweren wechselnde Bedienelemente den schnellen Zugriff. Wer unterwegs auf dem iPhone nachschlägt, kann die Kategorie häufig erst im Detail lesen. Fehlerbehebung wurde für diesen Ausschnitt als nicht anwendbar gewertet.

## Prüfnachweis

- **Demo-Einstieg:** `--demo-seed --select "Musterbank Girokonto"`. Auf dem Mac mit Standarddichte bei 1.300 pt beginnen. Für BUC-1 mit `--text-size larger --window-width 900` starten. Wegen der Mindestbreite waren es 1.237 pt. Der iPhone-Build verwendete in einer temporären Kopie den Swift-Fix aus [#1919](https://github.com/GiroCrab/GiroCrab/pull/1919).
- **BUC-1:** In der REWE-Zeile die Kategorie neben dem Tag ansehen (`booking-iphone-standard.png`, `booking-mac-gross.png`). In `BookingRow` teilt sich die einzeilige Kategorie die Breite mit `BookingTagMarkers`, deren Größe fixiert ist. Bei knapper Breite muss der Kategoriename erkennbar bleiben.
- **BUC-2:** Auf dem Mac auf „Kompakt“ wechseln und die Bäckerei-Zeile ohne Auswahl mit der Standardliste vergleichen (`booking-mac-kompakt.png`). `ViewThatFits` in `BookingRow` blendet die Direktwahl aus, wenn ihr vollständiges Label nicht passt. Die Kategorie muss auch in einer schmalen Kompaktzeile auffindbar bleiben, etwa als klarer kompakter Zugang.
- **BUC-3:** Auf dem Mac mit `--pick rewe` starten. In `booking-mac-auswahl.png` liegen `#B5474F` für „Supermarkt“ und `#D2DDEE` für die Auswahlfläche nach sRGB-Umrechnung bei ca. 3,9:1. `BookingCategoryLabel` nutzt `Color.readable`, während `RowSelectionBackground` den Hintergrund ändert. Abnahme ab 4,5:1 auf der normalen und der ausgewählten Zeile.
- **Schwärzung und Auswahl:** Beträge, persönliche Texte und Kategorien sind auf Mac und iPhone in den Aufnahmen mit `--redact=labels` verdeckt. 55 Swift-Tests für Dichte, Auswahl, Kategorie und Datenschutz bestanden. Der manuelle Mac-Durchgang für Tastaturfokus und Popover sowie VoiceOver stehen aus.
