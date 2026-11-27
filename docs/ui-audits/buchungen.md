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
| NEU-1 | P2 | Orientierung | iPhone | Bei „Accessibility XL“ sind der Kontoname und der darunter angeordnete Saldo vollständig lesbar | Fix [#2046](https://github.com/GiroCrab/GiroCrab/pull/2046) | `fixed` |

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

- **Schwärzung und Auswahl:** Beträge, persönliche Texte und Kategorien sind auf Mac und iPhone in den Aufnahmen mit `--redact=labels` verdeckt. 55 Swift-Tests für Dichte, Auswahl, Kategorie und Datenschutz bestanden. Der manuelle Mac-Durchgang für Tastaturfokus und Popover sowie VoiceOver stehen aus.
- **Stand der Direktwahl:** Dieses Audit enthält die in [#1890](https://github.com/GiroCrab/GiroCrab/pull/1890) eingeführte Kategorieauswahl.

#### BUC-1 · Kategoriename und Tag konkurrieren um Breite

- **Ort:** `BookingListView.swift` (`BookingRow`) und `TagsUI.swift` (`BookingTagMarkers`).
- **Beobachtung:** Beim REWE-Umsatz bleibt in der Standardliste auf dem iPhone „S…“, auf dem schmalen Mac mit großer Schrift „Super…“. Der Tag behält seine feste Breite.
- **Vorschlag:** Die Kategorie vor den optionalen Markern priorisieren. Bei wenig Platz einen Marker verdichten oder den Namen in eine eigene Zeile setzen.
- **Prüfung:** REWE mit Standardgröße am iPhone und am Mac mit `--text-size larger --window-width 900` prüfen. Die Kategorie muss erkennbar bleiben und der Betrag vollständig sichtbar sein.

#### BUC-2 · Kategorie-Direktwahl in der Kompaktliste fehlt

- **Ort:** `BookingListView.swift`, `ViewThatFits` in `BookingRow`.
- **Beobachtung:** Bei „Bäckerei Konditorei König“ fällt in der unselektierten Kompaktzeile die Direktwahl weg, wenn das volle Label nicht passt. Der Tag bleibt sichtbar.
- **Vorschlag:** Einen kompakten, eindeutig beschrifteten Zugang zur selben Kategorieauswahl als Alternative in `ViewThatFits` anbieten.
- **Prüfung:** Die Bäckerei bei „Kompakt“ ohne Auswahl öffnen. Die Kategorie muss direkt wählbar sein. Dieselbe Zeile bei Standarddichte vergleichen.

#### BUC-3 · Kategorietext hat zu wenig Kontrast bei Auswahl

- **Ort:** `BookingListView.swift`, `BookingCategoryLabel` und `RowSelectionBackground`.
- **Beobachtung:** Bei `--pick rewe` im Hellmodus liegen „Supermarkt“ (`#B5474F`) und die blaue Auswahlfläche (`#D2DDEE`) bei rund 3,9:1 nach sRGB. `Color.readable` berücksichtigt die Auswahlfläche nicht.
- **Vorschlag:** Die Textfarbe gegen den tatsächlich sichtbaren Hintergrund bestimmen oder in der Auswahl eine kontrastreiche Textfarbe verwenden.
- **Prüfung:** Hell und Dunkel bei ausgewählter und nicht ausgewählter REWE-Zeile vergleichen. Der Kategorietext erreicht jeweils mindestens 4,5:1.

### Buchungsinspektor

- **Nachprüfung:** 25.11.2026, `main` @ `09f11987`, INS-12 und INS-13. [Prüfbuild](README.md#prüfbuilds).
- **Umgebung:** macOS 27.2.1 und iPhone 18 Pro (iOS 27.2), Deutsch. Standard und große Schrift, Hell/Dunkel und Schwärzung nach [Prüfmatrix](../ui-audit.md#prüfmatrix).
- **Demo-Einstieg:** `--demo-seed --pick rewe`, Standardlayout im Inspektor. Für die Schwärzung `--redact` ergänzen.

- Der gesonderte Auswahlfehler beim Ansichtswechsel wurde mit [#1809](https://github.com/GiroCrab/GiroCrab/pull/1809) behoben. [Issue #1808](https://github.com/GiroCrab/GiroCrab/issues/1808)
- **Pilot und Nacharbeit:** Erster Abgleich nach #1666 auf `c96ddc36`. Die Entscheidung zu den Radien steht in [DESIGN.md](../../DESIGN.md) und im [RFC #1673](https://github.com/GiroCrab/GiroCrab/issues/1673). [Pilot #1707](https://github.com/GiroCrab/GiroCrab/pull/1707) · [Nacharbeit #1856](https://github.com/GiroCrab/GiroCrab/pull/1856)
- **Früherer Bericht:** Code-Stellen, Vorschläge und Prüfschritte zu INS-1 bis INS-13. [Ausführlicher Pilotbericht](https://github.com/GiroCrab/GiroCrab/blob/ba18ff6b47f2a7044cb1a12c16e12d49c94d2453/docs/ui-audits/buchungs-inspektor.md)

#### INS-6 · Uneinheitliche Radien

- **Ort:** `BookingDetailView.swift`, `CategorySuggestionBox` (8 pt), `CategoryChip` (7 pt) und der Editor im Bearbeiten-Popover (5 pt).
- **Beobachtung:** Die Radien fallen beim direkten Vergleich der Kategorieempfehlung, der Kacheln und des geöffneten Popovers unterschiedlich aus.
- **Vorschlag:** Das gemeinsame Set für diese Bedienelemente im RFC #1673 entscheiden und die passenden Tokens ansichtsübergreifend anwenden.
- **Prüfung:** Die drei Elemente im Inspektor und vergleichbare Bedienelemente in den betroffenen Ansichten gegen das beschlossene Set prüfen.

#### INS-12 · Heller Streifen am Notizfeld

- **Ort:** `BookingDetailView.swift`, `TextEditor` in `NoteCard` mit `.scrollContentBackground(.hidden)` und `Theme.systemSurface`.
- **Beobachtung:** Im Mac-Dunkelmodus bleibt rechts am leeren Notizfeld ein heller Streifen sichtbar. Kategorienflächen und iPhone-Sheet folgen inzwischen dem Erscheinungsbild.
- **Vorschlag:** Am nativen Editor und an seiner Scrollfläche prüfen, welche Fläche den Rand zeichnet. Die Fläche an den Systemhintergrund des Notizfelds anpassen.
- **Prüfung:** Das Notizfeld am Mac leer und mit Text in Hell und Dunkel prüfen. Der Rand bleibt gleichmäßig. Kategorien, iPhone-Sheet und geschwärzte Titel stichprobenartig nachprüfen.

#### INS-13 · Händler-Historie liegt unter dichten Auswahlflächen

- **Ort:** `TagsUI.swift` (`TagsCard`), `BookingDetailView.swift` (`CategoryGridCard`) und `InspectorLayout.swift`.
- **Beobachtung:** Bei Standardgröße stehen acht Vorschläge plus „+3 weitere“, elf Hauptkategorien und Unterkategorien vor der Historie. Beim Wechsel zwischen Buchungen kostet der Weg dorthin Scrollen.
- **Vorschlag:** Kompaktere oder erst bei Fokus sichtbare Vorschläge und ein kompakteres Raster als Varianten vergleichen. Die Kategorie-Direktwahl aus [#1890](https://github.com/GiroCrab/GiroCrab/pull/1890) berücksichtigen.
- **Prüfung:** Am Mac bei Standardgröße prüfen, ob die Historie ohne Scrollen erreichbar wird und wiederholtes Kategorisieren per Maus und Tastatur flüssig bleibt.

### Buchungssuche und Filter

- **Demo-Einstieg:** `--demo-seed --select "Musterbank Girokonto" --demo-filter expense`. Auf dem Mac „Sehr groß“ und schmale Fensterbreite wählen, auf dem iPhone „Accessibility XL“.

- **Ohne Treffer:** Auf dem iPhone im Suchfeld `zzzauditkeintreffer` eingeben. „Keine Treffer“, die eingegebene Suche und die Aktion zum Zurücksetzen sind vorhanden.

#### SUC-1 · Weitere Filter sind bei großer Schrift schwer auffindbar

- **Ort:** `BalanceHeaderView.swift`, `StreamHeaderCard.filterRow`.
- **Beobachtung:** Im Demo-Fall mit aktivem Ausgabenfilter endet die horizontal scrollende Zeile am schmalen Mac mitten in „Ta…“. Am iPhone bleiben Zeitraum und Status sichtbar. Betragsfilter und weitere Aktionen liegen ohne Scrollindikator außerhalb des Ausschnitts.
- **Vorschlag:** Bei großer Schrift einen sichtbaren Hinweis auf weitere Filter oder einen beschrifteten Überlaufzugang anbieten.
- **Prüfung:** Mit dem beschriebenen Demo-Start auf dem schmalen Mac und am iPhone bei „Accessibility XL“ den Betragsfilter öffnen. Der Zugangsweg muss erkennbar sein.

### Kontoübersicht und Kennzahlen

- **Demo-Einstieg:** `--demo-seed`, „Alle Konten“ öffnen. Mac „Sehr groß“ und `--window-width 900`. iPhone „Accessibility XL“. Dieselben Fälle mit `--select "Musterbank Girokonto"` vergleichen.

#### KON-1 · Kennzahlen kürzen Geldbeträge

- **Ort:** `BalanceHeaderView.swift`, `KPI`.
- **Beobachtung:** Die Beträge werden einzeilig verkleinert. In „Alle Konten“ erscheint am Mac eine gekürzte Konsumkennzahl wie „−4.947,21…“. Am iPhone sind Kennzahlen unter dem verfügbaren Betrag betroffen.
- **Vorschlag:** Bei wenig Breite weniger Kennzahlen pro Zeile anordnen und den Geldbetrag samt Währung zusammenhalten.
- **Prüfung:** „Alle Konten“ und das Girokonto bei großer Schrift am schmalen Mac und am iPhone vergleichen. Alle Kennzahlenbeträge bleiben vollständig lesbar.

#### KON-2 · Verfügbarer Betrag ist im Kopf gekürzt

- **Ort:** `BalanceHeaderView.swift`, kompakte Kopfzeile in `StreamHeaderCard`.
- **Beobachtung:** In „Alle Konten“ bleibt auf dem iPhone bei „Accessibility XL“ der verfügbare Betrag auf eine Zeile begrenzt und wird gekürzt.
- **Vorschlag:** Bei Platzmangel dem Betrag eine eigene Zeile geben und das Label getrennt anordnen.
- **Prüfung:** Am iPhone bei „Accessibility XL“ „Alle Konten“ öffnen. Vorzeichen, Betrag und Währung sind vollständig sichtbar.

#### KON-3 · Beschriftungen im Saldoverlauf überlappen

- **Ort:** `BalanceHeaderView.swift`, `BalanceChart`.
- **Beobachtung:** In „Alle Konten“ überlagern sich am iPhone bei „Accessibility XL“ skalierende Datums- und Betragsbeschriftungen in der kompakten Diagrammfläche.
- **Vorschlag:** Die Zahl der Achsenmarken bei großer Schrift reduzieren oder mehr Platz für die Achsen reservieren.
- **Prüfung:** Auf demselben iPhone „Alle Konten“ in Standardgröße und „Accessibility XL“ vergleichen. Daten und Werte bleiben einzeln lesbar.

### Neue Umsätze

- **Demo-Einstieg:** `--demo-seed --view neu`. Auf dem iPhone von Standardgröße auf „Accessibility XL“ wechseln. Die erste Kontogruppe vergleichen.

- **Schwärzung:** Im Mac-Durchgang `--redact=labels` verwenden. Für den Leerzustand alle neuen Umsätze des Demo-Tresors als gelesen markieren.

#### NEU-1 · Kontogruppen verlieren erkennbare Namen

- **Ort:** `NewBookingsView.swift`, `NewBookingsCardHeader`.
- **Beobachtung:** In „Neue Umsätze“ bleibt am iPhone bei „Accessibility XL“ von „Musterbank Girokonto“ nur „Mu…“. Die Zeile mit Bank und IBAN hilft kaum bei der Zuordnung; der Saldo bleibt lesbar.
- **Vorschlag:** Bei geringer Breite den Namen und den fest dimensionierten Saldo untereinander anordnen.
- **Prüfung:** Auf dem iPhone mit dem Demo-Einstieg Standard und „Accessibility XL“ vergleichen. Der erste Kontoname ist identifizierbar und der Saldo vollständig sichtbar.

- **Nachprüfung:** 27.11.2026, iPhone 18 Pro (Simulator, iOS 27.0), Deutsch, hell, „Accessibility XL“. Vollständige App mit Demodaten auf Basis von `eb94f16c` vor und nach dem Swift-Patch aus `835828f3`. „Musterbank Girokonto“ und der Saldo sind nachher vollständig lesbar. Bei Standardschrift war der Kontokopf pixelgleich. Bildvergleich in [#2046](https://github.com/GiroCrab/GiroCrab/pull/2046).
