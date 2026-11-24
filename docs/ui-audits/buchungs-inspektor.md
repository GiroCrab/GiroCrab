# UI-Audit: Buchungs-Inspektor

Erster Pilot nach [`docs/ui-audit.md`](../ui-audit.md) (RFC #1673).

- **Datum / Stand:** 22.11.2026, `main` @ `d93042a1`, nach #1666 (flache Hauptspalten) auf `c96ddc36` nachgeprüft
- **Plattformen:** macOS, iOS (iPhone-Sheet)
- **Dateien:** `GiroCrabApp/Sources/GiroCrabUI/BookingDetailView.swift` (Inspektor-Abschnitte),
  `GiroCrabApp/Sources/GiroCrabUI/BookingListView.swift` (iPhone-Sheet)
- **Ansicht:** rechte Spalte auf dem Mac (Inspektor: Beleg, Notiz, Tags, Wiederkehrend, Kategorie) und
  dieselbe Ansicht als Sheet auf dem iPhone
- **Werkzeug:** Impeccable `critique` und `audit` (nativ) mit `PRODUCT.md` / `DESIGN.md` als Kontext, ausgeführt von
  einem Agenten. Für native Apps gibt es keinen automatischen Detector (er liest nur Web-Code): Impeccable liefert
  die Methode (Heuristiken, Personas, Plattformregeln), die Beobachtungen stammen aus Code und Screenshots.
  Ergebnis: critique **25/36** (Heuristik 9 n/a, „Acceptable“), audit **14/20** („Good“).
- **Vorlauf:** Ein erster Durchgang mit der Checkliste aus `docs/ui-audit.md` (#1689) fand INS-1 bis INS-9.
  Impeccable deckt davon Schriftgröße, Bewegung, Kontrast und Hierarchie mit ab und fand INS-10 bis INS-13
  zusätzlich; nur die GiroCrab-Regeln (Schwärzung, Beträge) kennt es nicht von selbst.
- **Nachweis:** jede Beobachtung im Code und in Screenshots mit Demo-Daten bestätigt; macOS: Standard, „Sehr groß“,
  `--dark`, `--window-width 900`, `--redact`; iPhone: iOS-Simulator, normal und `--redact`.

| ID | Gefunden mit | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| INS-1 | Checkliste | Barrierefreiheit | macOS | Betrag folgt der Schriftgröße nicht | Fix | behoben |
| INS-2 | Checkliste | Barrierefreiheit | macOS | Feste Maße wachsen nicht mit; Kategorie-Kacheln brechen mitten im Wort um | Fix | behoben |
| INS-3 | Checkliste | Datenschutz, Dichte | iOS | iPhone-Sheet ohne Titel und ohne „Fertig“ | Fix | behoben |
| INS-4 | Checkliste, Impeccable | Hierarchie | beide | „Text bearbeiten …“ steht in einer eigenen Zeile über dem Inhalt | Issue | Issue anzulegen |
| INS-5 | Checkliste, Impeccable | Konsistenz | beide | IBAN einmal gleichbreit, einmal proportional gesetzt | Issue | Issue anzulegen |
| INS-6 | Checkliste | Konsistenz | beide | Drei kleine Radien in einer Ansicht (8 / 7 / 5 pt) | Entscheidung | im RFC #1673 |
| INS-7 | Checkliste | Doku | – | Keine Design-Seite für den Inspektor | Issue | Issue anzulegen |
| INS-8 | Checkliste | Datenschutz | beide | Händler-Domain bleibt in „Problem melden“ sichtbar | Verworfen | gewollt, siehe unten |
| INS-9 | Checkliste | Barrierefreiheit | beide | Umordnen der Abschnitte federt auch mit „Bewegung reduzieren“ | Fix | behoben |
| INS-10 | Impeccable | Barrierefreiheit | beide | Leeres Notizfeld ohne Platzhalter, Editor ohne VoiceOver-Label | Fix | behoben |
| INS-11 | Impeccable | Texte | beide | Unterkategorien brechen schon bei Standardgröße im Wort um | Fix | behoben |
| INS-12 | Impeccable | Hell und dunkel | beide | Gemischte Flächen (Theme-Slate auf Systemflächen); iPhone-Sheet dunkel im hellen Modus | Issue | Issue anzulegen |
| INS-13 | Impeccable | Hierarchie | beide | Tag-Vorschläge und Kategorie-Raster überwiegen den Inhalt | Issue | Issue anzulegen |

## INS-1: Betrag folgt der Schriftgröße nicht

**Ort:** `BookingDetailView.swift:184` (`ReceiptCard`) · **Beobachtung:** Der Betrag nutzt
`.font(.system(.largeTitle, …))` statt `.appFont`. Bei Schriftgröße „Sehr groß“ wachsen Händlername, Datum und
Verwendungszweck, der Betrag bleibt bei 26 pt und tritt im Verhältnis zurück.
`lint-fonts.sh` erkennt diese Form nicht. · **Vorschlag:** `.appFont(.largeTitle, weight: .semibold, design: .rounded,
monospacedDigit: true)`; Aussehen bei Standard unverändert. · **Prüfung:** Screenshot `--text-size larger`: Betrag
größer als bei Standard.

## INS-2: Feste Maße wachsen nicht mit

**Ort:** `BookingDetailView.swift` – `CategoryGridCard` (`GridItem(.adaptive(minimum: 104))` und `120`),
`NoteCard` (`minHeight: 54, maxHeight: 120`), Popover „Text bearbeiten“ (`height: 90`, `width: 340`) ·
**Beobachtung:** Bei „Sehr groß“ passen durch die festen Mindestbreiten drei statt zwei Kategorie-Kacheln
nebeneinander; die Namen brechen mitten im Wort um („Einnahme|n“, „Lebensmi|ttel“). Notizfeld und
Bearbeiten-Popover zeigen bei größerer Schrift weniger Zeilen. Regel: feste Maße mit Text werden mit
`textScale` multipliziert (`CLAUDE.md`, `DESIGN.md` › Layout). · **Vorschlag:** alle genannten Werte `* textScale`. ·
**Prüfung:** Screenshot `--text-size larger`: zwei Kacheln pro Zeile, keine Umbrüche im Wort.

## INS-3: iPhone-Sheet ohne Titel und ohne „Fertig“

**Ort:** `BookingListView.swift:146` · **Beobachtung:** Auf dem iPhone öffnet sich der Inspektor als Sheet mit
leerer Navigationsleiste. Es gibt keinen sichtbaren Weg zum Schließen außer Wischen und keinen Hinweis,
welche Buchung gezeigt wird, sobald man nach unten scrollt. · **Vorschlag:** Händlername als Titel (inline),
„Fertig“ als Bestätigungsaktion, sichtbarer Griff. Der Titel ist ein Parametertext und wird für „Problem melden“
über `redaction.masking` (aus `@Environment(\.reportRedaction)`) ersetzt (`.sensitive()` greift dort nicht). · **Prüfung:** iOS-Simulator; Bericht
„Problem melden“ zeigt den Titel geschwärzt.

## INS-4: „Text bearbeiten …“ in eigener Zeile

**Ort:** `BookingDetailView.swift:129` · **Beobachtung:** Die Aktion steht allein in einer Zeile oberhalb von
Händler und Betrag. Die erste Zeile des wichtigsten Abschnitts ist damit eine Nebenaktion, der Inhalt rutscht nach unten.
· **Vorschlag:** als Symbol-Button in die Kopfzeile neben den Händlernamen oder ins Kontextmenü; Varianten im
Issue zeigen. Auf dem iPhone stehen „Anpassen“ und „Text bearbeiten …“ sogar in zwei Zeilen vor dem Händler; in
der mittleren Sheet-Höhe bleibt dadurch kaum Platz für den Inhalt; bei sehr großer Schrift (Dynamic Type
„Accessibility XL“) ist der Betrag dort gar nicht mehr zu sehen. · **Prüfung:** Händlername ist die erste Zeile
des Abschnitts.

## INS-5: IBAN uneinheitlich gesetzt

**Ort:** `BookingDetailView.swift:172` und `:165` · **Beobachtung:** Ohne erkanntes Gegenkonto steht die IBAN
gleichbreit (`design: .monospaced`), mit erkanntem Gegenkonto proportional in der Zeile „⇄ Konto (IBAN)“.
· **Vorschlag:** IBAN in beiden Fällen gleich setzen, z. B. als eigenes `Text` mit `.monospaced`. · **Prüfung:**
Buchung mit und ohne eigenes Gegenkonto vergleichen.

## INS-6: Drei kleine Radien in einer Ansicht

**Ort:** Vorschlagsbox 8 pt, Kategorie-Kacheln 7 pt, Bearbeiten-Popover 5 pt. Vor #1666 kamen Karten (12 pt) und
Notizfeld (6 pt) dazu; beide sind jetzt ohne Radius.
**Beobachtung:** passt zum offenen Punkt in `DESIGN.md` › Shapes. **Weg:** keine Einzeländerung; im RFC über ein
festes Set entscheiden, dann ansichtsübergreifend umstellen.

## INS-7: Keine Design-Seite

**Beobachtung:** Für den Inspektor gibt es keine `docs/*_UI.md`-Seite, obwohl er die meistgenutzte Detailansicht
ist; Reihenfolge und Zweck der Abschnitte sind nur im Code beschrieben. **Vorschlag:** kurze Seite
`docs/INSPEKTOR_UI.md`, verlinkt aus dem Kommentar an `InspectorView`.

## INS-8: Händler-Domain bleibt sichtbar (verworfen)

**Beobachtung:** Mit `--redact` ist der Händlername geschwärzt, die Domain („rewe.de · Wörterbuch“) bleibt lesbar.
**Prüfung:** `MerchantDomainRow` schwärzt erratene und eigene Domains, nicht aber Domains aus dem Händler-Wörterbuch:
Diese gehören zu bekannten Marken, deren Logo ohnehin sichtbar bleibt (`CLAUDE.md` › Sensitive UI). Kein Befund.

## INS-9: Animation ohne „Bewegung reduzieren“

**Ort:** `BookingDetailView.swift:17` (`InspectorView.updateLayout`) · **Beobachtung:** Beim Umordnen und
Ausblenden der Abschnitte läuft `withAnimation(.snappy(…))` direkt; mit „Bewegung reduzieren“ federn die Abschnitte
trotzdem. Gefunden mit dem Code-Hinweis aus der Checkliste. · **Vorschlag:**
`Motion.animation(.snappy(duration: 0.2))` wie im Rest der App. · **Prüfung:** mit „Bewegung reduzieren“
springen die Abschnitte ohne Federn an ihren Platz.

## INS-10: Leeres Notizfeld ohne Platzhalter und Label

**Ort:** `BookingDetailView.swift` (`NoteCard`) · **Beobachtung:** Ohne Notiz zeigt der Abschnitt eine leere,
randlose Fläche, die wie eine Schwärzung wirkt; VoiceOver kündigt den Editor ohne Bezug zur Überschrift „Notiz“
an. · **Vorschlag:** Platzhalter „Notiz hinzufügen …“ im leeren, nicht fokussierten Feld; `accessibilityLabel`
„Notiz“. · **Prüfung:** Screenshot zeigt den Platzhalter; er erscheint nur bei leerer Notiz, `.sensitive()` am Editor bleibt unverändert.

## INS-11: Unterkategorien brechen im Wort um

**Ort:** `BookingDetailView.swift` (`CategoryGridCard`, zweite Ebene) · **Beobachtung:** Schon bei Standardgröße
reichen 120 pt nicht für „Lebensmittelrettung“ mit Symbol; der Name bricht zu „Lebensmittelrettun|g“ um. INS-2
deckte nur das Mitwachsen ab. · **Vorschlag:** Mindestbreite 150 pt (mit `textScale`); im üblichen Inspektor
steht die Ebene dann einspaltig. · **Prüfung:** Screenshot Standard und „Sehr groß“ ohne Umbruch im Wort.

## INS-12: Flächen und Erscheinungsbild

**Ort:** `CategoryChip` (`BookingDetailView.swift:763`), `CategorySuggestionBox` (`:695`) · **Beobachtung:** Seit #1666
steht der Inspektor auf `Theme.systemBackground` (neutral). Kacheln und Vorschlagsbox füllen weiter mit
`Theme.card` / `Theme.canvas` (Slate); im Dunkelmodus erscheinen blaue Blöcke auf grauem Grund. Im Notizfeld zeigt
der Dunkelmodus am rechten Rand einen helleren Streifen. · **Vorschlag:** Flächen im Inspektor auf `Theme.system*`
umstellen, gemeinsam mit den übrigen Ansichten aus #1666 prüfen. · **Prüfung:** Screenshot `--dark`, keine Slate-Flächen.

Auf dem iPhone ist das Sheet dunkel, obwohl das System auf hell steht und die Ansicht dahinter hell ist (iOS-Simulator,
auch auf `main` ohne die Änderung aus INS-3). Ursache noch offen; gehört zum selben Issue, weil es dieselbe Frage
betrifft: welche Flächen und welches Erscheinungsbild der Inspektor hat.

## INS-13: Auswahl-Werkzeuge überwiegen den Inhalt

**Beobachtung:** Unter dem Beleg folgen acht farbige Tag-Vorschläge plus „+3 weitere“ (angewandte Tags unterscheiden
sich nur durch das „×“), danach 11 Kategorie-Kacheln, die Unterkategorien und zwei große Schaltflächen. Wer die
Buchungen per Tastatur kategorisiert (C / R in der Liste), scrollt bei jeder Buchung an dieser Fläche vorbei. ·
**Vorschlag:** Tag-Vorschläge gedämpft oder erst bei Fokus zeigen; Kategorie-Auswahl kompakter; Varianten im Issue
zeigen. · **Prüfung:** Händler-Historie ohne Scrollen sichtbar bei Standardgröße.

## Kleinere Beobachtungen (Impeccable, ohne Weg)

- Die Abschnittsüberschrift „Kategorie“ hat als einzige kein Symbol.
- „Wörterbuch“ neben der Händler-Domain ist ohne Erklärung Fachsprache.
- „Geprüft“ ist auf macOS ein Schalter; für das Abhaken einer Buchung wäre eine Checkbox üblicher.
- iPhone, sehr große Schrift: die Händler-Domain bricht zwischen „rewe.“ und „de“ um.

## Hell und dunkel, schmales Fenster

Im Vorlauf ohne Befund: Im Dunkelmodus tragen Betrag, Kategorie-Kacheln und Tags dieselben Bedeutungen
wie hell (die gemischten Flächen fand erst Impeccable, INS-12); bei 900 pt Fensterbreite und „Sehr groß“ bleiben Abschnitte, Texte und Kacheln lesbar. Daten stehen
im Inspektor einzeln (kein Spaltenvergleich), `monospacedDigit` ist dort nicht nötig.

## Prüfung der Fixes

- macOS, Schriftgröße „Sehr groß“: Betrag wächst mit, Kategorie-Raster zweispaltig ohne Umbruch im Wort,
  Notizfeld höher (Vorher/Nachher-Screenshots im PR, macOS und iPhone).
- macOS, `--redact`: Betrag und Notiz weiter geschwärzt.
- macOS, Standard und „Sehr groß“: Notizfeld mit Platzhalter (INS-10), Unterkategorien einspaltig ohne Umbruch
  im Wort (INS-11).
- iPhone (iOS-Simulator, Demo-Daten): Sheet mit Titel „REWE“, „Fertig“ und Griff; mit `--redact` erscheint der
  Titel als „•••“; dazu dunkel und Dynamic Type „Accessibility XL“. Zum Öffnen des Sheets ohne Antippen wurde
  `--pick` auf das Sheet erweitert (siehe `docs/ui-audit.md` › iPhone).
- `swift test`: 1149 Tests (nach #1666), 0 Fehler; `lint-fonts`, `lint-report-dialogs`, `lint-vault-access`, `lint-ios-compat` grün.

## Außerhalb des Bereichs bemerkt

Nicht geprüft, nur notiert für spätere Audits: Bei „Sehr groß“ wird in der Filterzeile über der Buchungsliste
„Betrag“ zu „Betra“ gekürzt; in der Seitenleiste bricht „41 % getilgt“ beim Autokredit auf zwei Zeilen um.
Bei „Sehr groß“ werden in der Kennzahl-Leiste über der Buchungsliste die Beträge gekürzt („−4.947,21…“,
widerspricht „Nie abschneiden“), nach #1666 schon bei normaler Fensterbreite; „Cashflow“ bricht mitten im Wort um.
