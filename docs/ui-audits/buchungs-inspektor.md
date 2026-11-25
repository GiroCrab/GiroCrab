# UI-Audit: Buchungs-Inspektor

- **Datum / Stand:** 22.11.2026, `main` @ `d93042a1`.
- **Ansicht:** Buchungsdetail in der rechten Mac-Spalte und im Sheet auf dem iPhone.
- **Dateien:** `BookingDetailView.swift`, `TagsUI.swift`, `RootView.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` plus GiroCrab-Checkliste.
- **Nachweis:** Demo-Aufnahmen in Hell, Dunkel, bei großer Schrift und mit Schwärzung. Erster Abgleich nach #1666 auf `c96ddc36`. [Pilot #1707](https://github.com/GiroCrab/GiroCrab/pull/1707) · [Nacharbeit #1856](https://github.com/GiroCrab/GiroCrab/pull/1856)

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
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

Die Prioritäten und Befund-IDs stammen aus dem Pilot. Die gemeinsamen Radien sind in [DESIGN.md](../../DESIGN.md) und
im [RFC #1673](https://github.com/GiroCrab/GiroCrab/issues/1673) vermerkt.

## Impeccable-Bewertung

**Critique 25/36 · Technisches Audit 14/20.** Im Pilot fielen vor allem die dichten Auswahlbereiche,
die unterschiedlichen Flächen und die Barrierefreiheit des Notizfelds auf.

## Prüfnachweis

- **Nachprüfung:** 25.11.2026, `main` @ `09f11987`, INS-12 und INS-13. Für iOS lag der Swift-Buildfix aus [#1919](https://github.com/GiroCrab/GiroCrab/pull/1919) in einer temporären Kopie.
- **Umgebung:** macOS 27.2.1 und iPhone 18 Pro (iOS 27.2), Deutsch. Standard und große Schrift, Hell/Dunkel und Schwärzung nach [Prüfmatrix](../ui-audit.md#prüfmatrix).
- **Demo-Einstieg:** `--demo-seed --pick rewe`, Standardlayout im Inspektor. Für die Schwärzung `--redact` ergänzen.
- **INS-12:** In `NoteCard` den rechten Rand des leeren `TextEditor` im Mac-Dunkelmodus ansehen. Der Streifen ist in `mac-inspektor-dunkel.png` sichtbar. Auf dem iPhone ist das Sheet im hellen Modus hell. Titel, Name, Konto, Betrag und Buchungstext sind in der geschwärzten Aufnahme verdeckt.
- **INS-13:** Bei Standardgröße stehen acht Vorschläge plus „+3 weitere“, elf Hauptkategorien und Unterkategorien vor der Händler-Historie (`mac-inspektor-hell.png`). `TagsCard`, `CategoryGridCard` und die Reihenfolge in `InspectorLayout` prüfen. Bei einer Änderung den Weg zur Historie und die Kategorisierung mehrerer Buchungen per Maus und Tastatur messen. Dabei [#1890](https://github.com/GiroCrab/GiroCrab/pull/1890) berücksichtigen.
- Der gesonderte Auswahlfehler beim Ansichtswechsel wurde mit [#1809](https://github.com/GiroCrab/GiroCrab/pull/1809) behoben. [Issue #1808](https://github.com/GiroCrab/GiroCrab/issues/1808)
