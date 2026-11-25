# UI-Audit: Buchungen

- **Ansicht:** Buchungsliste und Inspektor, Suche und Filter, Kontoübersicht mit Kennzahlen sowie „Neue Umsätze“.
- **Dateien:** `BookingListView.swift`, `BalanceHeaderView.swift`, `TagFilterUI.swift`, `PlanningUI.swift`, `NewBookingsView.swift`, `NewBookings.swift`, `BookingRowText.swift`, `BookingCategoryMenu.swift`, `TagsUI.swift`, `BookingDetailView.swift`, `RootView.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` anhand von SwiftUI-Code und Demo-Aufnahmen, dazu GiroCrab-Checkliste.
- **Nachweis:** macOS 27.2.1, iPhone 18 Pro und Pro Max sowie iPad (iOS 27.2), Deutsch. Standard, große Schrift und Schwärzung mit Demo-Daten. [Prüfbuild vom 25.11.2026](README.md#prüfbuilds).

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| BUC-1 | P2 | Lesbarkeit | iPhone, macOS | In der Standardliste wird „Supermarkt“ auf dem iPhone zu „S…“. Auf dem schmalen Mac mit großer Schrift bleibt „Super…“. Der Tag daneben beansprucht weiter Platz | Issue | `open` |
| BUC-2 | P2 | Auffindbarkeit | macOS | In der Kompaktliste fehlt bei „Bäckerei Konditorei König“ die Kategorie-Direktwahl, obwohl der Tag angezeigt wird | Issue | `open` |
| BUC-3 | P2 | Kontrast | macOS | „Supermarkt“ erreicht auf dem blauen Hintergrund der ausgewählten REWE-Zeile in Hell etwa 3,9:1 | Issue | `open` |
| INS-1 | – | Barrierefreiheit | macOS | Betrag folgte der eingestellten Schriftgröße nicht | Fix | `fixed` |
| INS-2 | – | Barrierefreiheit | macOS | Feste Maße für Kategorie-Kacheln, Notiz und Bearbeiten-Popover wuchsen nicht mit | Fix | `fixed` |
| INS-3 | – | Orientierung, Datenschutz | iPhone | Das Sheet hatte weder Titel noch „Fertig“. Der ergänzte Titel wird geschwärzt | Fix | `fixed` |
| INS-4 | – | Hierarchie | macOS, iOS | „Text bearbeiten …“ stand über dem Beleg. Der Stift steht jetzt neben dem Händlernamen | Fix | `fixed` |
| INS-5 | – | Konsistenz | macOS, iOS | Die IBAN war bei erkanntem Gegenkonto proportional und sonst gleichbreit | Fix | `fixed` |
| INS-6 | – | Konsistenz | macOS, iOS | Die Bedienelemente nutzen mehrere kleine Radien. Die Vereinheitlichung ist ansichtsübergreifend offen | Entscheidung | `decision` |
| INS-7 | – | Dokumentation | – | Die [Design-Seite](../INSPEKTOR_UI.md) fehlte | Fix | `fixed` |
| INS-8 | – | Datenschutz | macOS, iOS | Die sichtbare Domain stammt aus dem Händler-Wörterbuch. Eigene und erratene Domains werden geschwärzt | Verworfen | `dismissed` |
| INS-9 | – | Barrierefreiheit | macOS, iOS | Umordnen der Abschnitte federte mit „Bewegung reduzieren“ | Fix | `fixed` |
| INS-10 | P2 | Barrierefreiheit | macOS, iOS | Dem leeren Notizfeld fehlten Platzhalter und VoiceOver-Label | Fix | `fixed` |
| INS-11 | P2 | Lesbarkeit | macOS, iOS | Unterkategorien wie „Lebensmittelrettung“ brachen bei Standardgröße mitten im Wort um | Fix | `fixed` |
| INS-12 | P1 | Erscheinungsbild | macOS, iOS | Karten und Sheet folgen dem Erscheinungsbild. Rechts am Notizfeld bleibt auf dem Mac ein heller Streifen sichtbar | Fix | `open` |
| INS-13 | P2 | Hierarchie | macOS, iOS | Tag-Vorschläge und Kategorie-Raster nehmen viel Platz vor der Händler-Historie ein | Issue | `open` |
| SUC-1 | P2 | Auffindbarkeit | macOS, iPhone | Bei großer Schrift endet die Filterzeile mitten in einem Bedienelement. Betragsfilter und weitere Aktionen liegen rechts außerhalb des sichtbaren Bereichs. Ein Scrollindikator fehlt | Issue | `open` |
| KON-1 | P1 | Lesbarkeit | macOS, iPhone | Kennzahlen zeigen bei großer Schrift abgeschnittene Geldbeträge. Auf dem Mac wird in „Alle Konten“ etwa „−4.947,21…“ angezeigt | Issue | `open` |
| KON-2 | P1 | Lesbarkeit | iPhone | Der verfügbare Betrag im Kopf von „Alle Konten“ ist mit „Accessibility XL“ gekürzt | Issue | `open` |
| KON-3 | P2 | Adaptivität | iPhone | Datums- und Betragsbeschriftungen im Saldoverlauf überlappen bei „Accessibility XL“ | Issue | `open` |
| NEU-1 | P2 | Orientierung | iPhone | Bei „Accessibility XL“ ist der erste Kontogruppentitel „Musterbank Girokonto“ nur noch als „Mu…“ erkennbar. Auch die zweite Zeile mit Bank und IBAN trägt kaum noch zur Zuordnung bei | Issue | `open` |

## Impeccable-Bewertung

| Ansicht | Datum / Stand | Critique | Technisches Audit | Kurzbewertung |
|---|---|---|---|---|
| Buchungsliste | 25.11.2026, `main` @ `09f11987` | 25/36 | 12/20 | Tagesgruppen, gleichbreite Beträge und Auswahlmarkierung unterstützen das Scannen. Beim monatlichen Kategorisieren am Mac erschweren wechselnde Bedienelemente den schnellen Zugriff. Wer unterwegs auf dem iPhone nachschlägt, kann die Kategorie häufig erst im Detail lesen. Fehlerbehebung wurde für diesen Ausschnitt als nicht anwendbar gewertet. |
| Buchungsinspektor | 22.11.2026, `main` @ `d93042a1` | 25/36 | 14/20 | Im Pilot fielen vor allem die dichten Auswahlbereiche, die unterschiedlichen Flächen und die Barrierefreiheit des Notizfelds auf. |
| Buchungssuche und Filter | 25.11.2026, `main` @ `09f11987` | 26/40 | 13/20 | Suche, Zeitraum und aktive Filter sind schnell erreichbar, solange sie in die sichtbare Zeile passen. Auf dem iPhone wird diese Auswahl bei großer Schrift schwer auffindbar. Die fachliche Kategorieauswahl ist Gegenstand von [PR #1896](https://github.com/GiroCrab/GiroCrab/pull/1896). |
| Kontoübersicht und Kennzahlen | 25.11.2026, `main` @ `09f11987` | 24/36 | 11/20 | Saldo und Zeitverlauf geben bei Standardgröße schnell Orientierung. Bei großer Schrift verliert die Übersicht zentrale Zahlen und Beschriftungen. Die Ansicht zeigt die Zahlen eines geladenen Demo-Tresors, ohne eigenen Fehlerfall im geprüften Ablauf. |
| Neue Umsätze | 25.11.2026, `main` @ `09f11987` | 27/36 | 14/20 | Die Gruppierung spart bei mehreren Konten den Wechsel zwischen Listen. Bei großer Schrift verliert gerade die Gruppenüberschrift ihre Zuordnungsfunktion. Ein Fehlerzustand wurde im geprüften Ablauf nicht ausgelöst. |

## Prüfnachweis

### Buchungsliste

- **Demo-Einstieg:** `--demo-seed --select "Musterbank Girokonto"`. Auf dem Mac mit Standarddichte bei 1.300 pt beginnen. Für BUC-1 mit `--text-size larger --window-width 900` starten. Wegen der Mindestbreite waren es 1.237 pt.
- **BUC-1:** In der REWE-Zeile die Kategorie neben dem Tag ansehen (`booking-iphone-standard.png`, `booking-mac-gross.png`). In `BookingRow` teilt sich die einzeilige Kategorie die Breite mit `BookingTagMarkers`, deren Größe fixiert ist. Bei knapper Breite muss der Kategoriename erkennbar bleiben.
- **BUC-2:** Auf dem Mac auf „Kompakt“ wechseln und die Bäckerei-Zeile ohne Auswahl mit der Standardliste vergleichen (`booking-mac-kompakt.png`). `ViewThatFits` in `BookingRow` blendet die Direktwahl aus, wenn ihr vollständiges Label nicht passt. Die Kategorie muss auch in einer schmalen Kompaktzeile auffindbar bleiben, etwa als klarer kompakter Zugang.
- **BUC-3:** Auf dem Mac mit `--pick rewe` starten. In `booking-mac-auswahl.png` liegen `#B5474F` für „Supermarkt“ und `#D2DDEE` für die Auswahlfläche nach sRGB-Umrechnung bei ca. 3,9:1. `BookingCategoryLabel` nutzt `Color.readable`, während `RowSelectionBackground` den Hintergrund ändert. Abnahme ab 4,5:1 auf der normalen und der ausgewählten Zeile.
- **Schwärzung und Auswahl:** Beträge, persönliche Texte und Kategorien sind auf Mac und iPhone in den Aufnahmen mit `--redact=labels` verdeckt. 55 Swift-Tests für Dichte, Auswahl, Kategorie und Datenschutz bestanden. Der manuelle Mac-Durchgang für Tastaturfokus und Popover sowie VoiceOver stehen aus.
- **Stand der Direktwahl:** Dieses Audit enthält die in [#1890](https://github.com/GiroCrab/GiroCrab/pull/1890) eingeführte Kategorieauswahl.

### Buchungsinspektor

- **Nachprüfung:** 25.11.2026, `main` @ `09f11987`, INS-12 und INS-13. [Prüfbuild](README.md#prüfbuilds).
- **Umgebung:** macOS 27.2.1 und iPhone 18 Pro (iOS 27.2), Deutsch. Standard und große Schrift, Hell/Dunkel und Schwärzung nach [Prüfmatrix](../ui-audit.md#prüfmatrix).
- **Demo-Einstieg:** `--demo-seed --pick rewe`, Standardlayout im Inspektor. Für die Schwärzung `--redact` ergänzen.
- **INS-12:** In `NoteCard` den rechten Rand des leeren `TextEditor` im Mac-Dunkelmodus ansehen. Der Streifen ist in `mac-inspektor-dunkel.png` sichtbar. Auf dem iPhone ist das Sheet im hellen Modus hell. Titel, Name, Konto, Betrag und Buchungstext sind in der geschwärzten Aufnahme verdeckt.
- **INS-13:** Bei Standardgröße stehen acht Vorschläge plus „+3 weitere“, elf Hauptkategorien und Unterkategorien vor der Händler-Historie (`mac-inspektor-hell.png`). `TagsCard`, `CategoryGridCard` und die Reihenfolge in `InspectorLayout` prüfen. Bei einer Änderung den Weg zur Historie und die Kategorisierung mehrerer Buchungen per Maus und Tastatur messen. Dabei [#1890](https://github.com/GiroCrab/GiroCrab/pull/1890) berücksichtigen.
- Der gesonderte Auswahlfehler beim Ansichtswechsel wurde mit [#1809](https://github.com/GiroCrab/GiroCrab/pull/1809) behoben. [Issue #1808](https://github.com/GiroCrab/GiroCrab/issues/1808)
- **Pilot und Nacharbeit:** Erster Abgleich nach #1666 auf `c96ddc36`. Die Entscheidung zu den Radien steht in [DESIGN.md](../../DESIGN.md) und im [RFC #1673](https://github.com/GiroCrab/GiroCrab/issues/1673). [Pilot #1707](https://github.com/GiroCrab/GiroCrab/pull/1707) · [Nacharbeit #1856](https://github.com/GiroCrab/GiroCrab/pull/1856)

### Buchungssuche und Filter

- **Demo-Einstieg:** `--demo-seed --select "Musterbank Girokonto" --demo-filter expense`. Auf dem Mac „Sehr groß“ und schmale Fensterbreite wählen, auf dem iPhone „Accessibility XL“.
- **SUC-1:** Auf dem iPhone bleiben Zeitraum und das Symbol für den aktiven Status sichtbar. Auf dem Mac endet die Zeile bei „Ta…“ vor dem Buchungszähler (`iphone-gross-filter.png`, `mac-filter-gross.png`). `StreamHeaderCard.filterRow` scrollt die Filter horizontal ohne Indikator. Bei großer Schrift muss der Zugang zu den weiteren Filtern erkennbar sein.
- **Ohne Treffer:** Auf dem iPhone im Suchfeld `zzzauditkeintreffer` eingeben. „Keine Treffer“, die eingegebene Suche und die Aktion zum Zurücksetzen sind vorhanden (`iphone-filter-ohne-treffer.png`).

### Kontoübersicht und Kennzahlen

- **Demo-Einstieg:** `--demo-seed`, „Alle Konten“ öffnen. Mac „Sehr groß“ und `--window-width 900`. iPhone „Accessibility XL“. Dieselben Fälle mit `--select "Musterbank Girokonto"` vergleichen.
- **KON-1 und KON-2:** `KPI` und die kompakte Kopfzeile in `StreamHeaderCard` erzwingen eine Zeile und skalieren den Betrag herunter. In `mac-konto-gross.png` ist die Konsumkennzahl gekürzt, in `iphone-gross-konto.png` der verfügbare Betrag und die Kennzahlen darunter. Beträge müssen samt Währung vollständig lesbar sein.
- **KON-3:** Beim Saldoverlauf in `iphone-gross-konto.png` überlagern sich Achsenwerte. `BalanceChart` verwendet skalierende Beschriftungen in einer kompakten Diagrammfläche. Abnahme mit getrennt lesbaren Achsenwerten bei derselben Schriftstufe.

### Neue Umsätze

- **Demo-Einstieg:** `--demo-seed --view neu`. Auf dem iPhone von Standardgröße auf „Accessibility XL“ wechseln. Die erste Kontogruppe vergleichen (`iphone-standard-neu.png`, `iphone-gross-neu.png`).
- **NEU-1:** `NewBookingsCardHeader` setzt Kontoname und IBAN in je eine Zeile neben den fest dimensionierten Betrag. Der Saldo bleibt lesbar. Abnahme mit einem identifizierbaren Kontonamen und vollständig sichtbarem Saldo, gegebenenfalls in getrennten Zeilen.
- **Schwärzung:** Im Mac-Durchgang `--redact=labels` verwenden. Für den Leerzustand alle neuen Umsätze des Demo-Tresors als gelesen markieren.
