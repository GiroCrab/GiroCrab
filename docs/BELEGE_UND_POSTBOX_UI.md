# Belege & Postbox (UI-Aufbau)

Diese Seite beschreibt, was die Ansicht „Belege & Postbox“ (`DocumentsView` in `DocumentsUI.swift`) zeigt, was sie
nicht tut und welche Regeln für Änderungen gelten. Sie gilt für macOS und iPhone/iPad; „Mobile Ansicht“ nennt die
Unterschiede. Wie Dokumente einer Buchung zugeordnet werden, steht in `docs/dokument-zuordnung.md`.

## Grundprinzip

Die Ansicht zeigt *Dokumente* (PDF, Fotos), keine Buchungen. Ein Dokument gehört zu einem Konto und kann zusätzlich
an genau einer Buchung hängen (`booking_documents.booking_id`). Zwei Herkünfte, die die Liste auseinanderhält
(`DocumentInfo.originTitle`): **Beleg** (selbst hinzugefügt: Drag & Drop, „Beleg hinzufügen …“) und **Postbox**
(von einer Bank geliefert, auch wenn es schon an einer Buchung hängt). Der Typ (Abrechnung, Kontoauszug, Steuer,
Postbox, Beleg) kommt vom Connector und zeigt sich am Symbol und im Typfilter.

**Aufbau (Mac):** Seitenleiste · Liste (links, mit Filtern) · Vorschau (Mitte). Die Ansicht hat **keine rechte
Buchungsspalte**: Sie hat ihre eigene Vorschau, und die Buchungsspalte würde sonst die zuletzt gewählte Buchung zeigen,
egal welches Dokument offen ist (`MainSplitView`). Auf iPhone/iPad öffnet ein Tipp auf eine Zeile die Vorschau als Blatt.

**Was die Ansicht nicht tut:** Sie ordnet nichts selbst zu und liest keine PDFs; Typ und Zuordnung kommen aus dem
Core und den Connectoren. Sie zeigt keine Buchungsliste und keine Buchungsfilter, und sie speichert keine Dokumente
außer über den Import („Beleg hinzufügen …“, Ablegen).

## Liste und Filter

Die Liste kommt aus einem Core-Aufruf (`allDocuments`, `list_all_documents`), der nach Konto, Suche und „Nur ohne
zugeordneten Umsatz“ filtert. **Typ**, **Jahr** und **Nur ungelesene** filtern danach im Speicher
(`DocumentFilters.apply`): Die Liste ist ohnehin geladen, und die Auswahl der Typen und Jahre richtet sich nach dem,
was es gibt. Über der Liste steht „N Dokumente“ oder „N von M Dokumenten“.

**Aufbau der Filter (Mac, iPad):** wie ein macOS-Formular (#1543). Eine Spalte mit rechtsbündigen Beschriftungen
(Bankzugang, Konto, Typ, Jahr), daneben die Auswahlfelder, die alle an derselben Kante beginnen und die restliche
Breite füllen; die Beschriftung steht auf der Grundlinie des Feldtexts und wird mit ihrem Feld abgeblendet (Konto bei
Bankzugang „Ohne Konto“). Jede Beschriftungszelle enthält alle Beschriftungen der Spalte unsichtbar übereinander und ist
damit so breit wie die längste (Schriftgröße, Sprache), ohne Messen und ohne Zustand. VoiceOver liest die Beschriftung
des Pickers selbst, die sichtbare ist ausgeblendet.

Typ und Jahr erscheinen, wenn es eine Wahl gibt oder ihr Filter gesetzt ist. Der gesetzte Wert bleibt auswählbar,
auch wenn die gefundenen Dokumente ihn nicht mehr haben (anderes Konto, neue Suche): Ein Picker braucht einen Eintrag
für seine Auswahl, sonst zeigt er nichts an und der Filter wirkt unsichtbar weiter
(`DocumentFilters.types/years(of:active:)`).

Darunter „Nur ungelesene“ und „Nur ohne zugeordneten Umsatz“ als Ankreuzfelder (Filter sind keine Einstellungen:
HIG › Toggles), dann „Beleg hinzufügen …“, der Export und das Menü „Aktionen“ (…) mit „Alle als gelesen markieren“ und
„Alle Dokumente anzeigen“ (setzt alle Filter zurück).

**Zeile:** Titel, „Herkunft · Konto“ und als dritte Zeile der zugeordnete Umsatz. „Ohne zugeordneten Umsatz“ (orange)
steht an eigenen Belegen, die auf ihren Umsatz warten; Bankdokumente haben meist keinen und zeigen ihn nur, solange
„Nur ohne zugeordneten Umsatz“ an ist (grau). Auf der blauen Auswahl (Liste mit Fokus im aktiven Fenster) zeichnet die
Zeile mit den hierarchischen Stilen (`.primary`/`.secondary`), die macOS dort hell macht; die festen Theme-Farben blieben
dunkel. Die graue Auswahl ohne Fokus behält die Theme-Farben. Eine macOS-Liste setzt `backgroundProminence` nicht, daher
merkt sich `DocumentsView` den Fokus der Liste (`listFocused`); iOS setzt es.

Die Liste liest nur den Index `booking_documents_info`, nie die PDF-Daten (diese sind Megabytes groß). Jede Spalte,
die die Liste zeigt, muss deshalb im Index stehen (Schema-Schritt 30 hat `seen_at` dort ergänzt).

## Gelesen-Status

Wie „Neue Umsätze“ bei Buchungen: Jedes Dokument ist gelesen oder ungelesen (`booking_documents.seen_at`, NULL =
ungelesen).

- **Neu angelegt:** Ein Dokument mit Datum der letzten 45 Tage (`UNSEEN_DAYS`) ist ungelesen, ein älteres kommt gelesen
  an: Nachgeladene Historie und Archive dürfen den Zähler nicht fluten. Von Hand hinzugefügte (Ablegen, „Beleg
  hinzufügen …“) sind sofort gelesen. Bestehende Dokumente zählen nach dem Update auf Schema 30 als gelesen.
- **Gelesen** wird ein Dokument, sobald es in der Liste gewählt wird. Von Hand: „Als gelesen / ungelesen markieren“
  (Kontextmenü), „Alle als gelesen markieren“ (Menü „Aktionen“ neben dem Export; für die sichtbare Liste, also mit
  Filtern) und im Kontextmenü von
  „Belege & Postbox“ in der Seitenleiste (für alle).
- **Anzeige:** Zähler an „Belege & Postbox“ in der Seitenleiste (`unseenDocumentCount`, `COUNT(*)` über einen
  Teilindex), blauer Punkt und fetter Titel an ungelesenen Zeilen, Filter „Nur ungelesene“. Das gerade gewählte Dokument
  bleibt in diesem Filter stehen, auch wenn es eben gelesen wurde.
- **Zähler der Seitenleiste** (Geplant, Lastschriften, ungelesene Dokumente) werden bei jedem Tresorwechsel und nach
  jeder Änderung neu gelesen; ein älterer Lesevorgang, der spät fertig wird, überschreibt keinen neueren
  (`sidebarCountsGeneration`).

## Export

„N exportieren …“ exportiert genau die Dokumente, die in der Liste stehen (nach Jahren sortiert); ohne Filter heißt der
Knopf „Alle exportieren …“. Ist ein Konto gewählt, legt der Export einen Ordner mit dem Kontonamen im gewählten Ordner
an, sonst liegen die Jahresordner direkt darin. Eine leere Liste exportiert nichts (der Knopf ist dann ausgeschaltet;
eine leere ID-Liste hieße im Core „alle“).

## Mobile Ansicht (iPhone, kompakte Breite)

Die Filter stehen untereinander ohne Beschriftungsspalte (die Auswahlfelder zeigen auf dem iPhone nur ihren Wert), die
Filter „Nur …“ sind Schalter (iOS hat keine Ankreuzfelder), die Titel sind linksbündig und
dürfen umbrechen. Ein Tipp auf eine Zeile öffnet die Vorschau als Blatt und markiert das Dokument als gelesen; die
Auswahl wird danach zurückgesetzt, damit dasselbe Dokument erneut geöffnet werden kann.

## Regeln für Änderungen

- **Schrift:** `.appFont(...)`, nicht `.font(...)` (`scripts/lint-fonts.sh`); feste Maße mit Text über `textScale`.
- **Datenschutz:** Titel, Konto, Buchungsbezug und Vorschau mit `.sensitive()`; „Händler · Betrag“ mit
  `.sensitive([.personal, .amounts])`. Sheets mit `.capturableSheet`.
- **Leistung:** Core-Aufrufe über 16 ms über `model.vaultOffMain`. Zählen über SQL (`unseen_document_count`), nie über die
  Liste; keine neue Spalte in der Liste ohne sie in den Index `booking_documents_info` aufzunehmen.
- **Neue Zustände** (z. B. ein weiterer Filter) gehören in `DocumentFilters` (rein, getestet in `DocumentFiltersTests`),
  nicht in die View.
- **Texte:** keine Marken fremder Software; alle Strings in `Localizable.xcstrings` (DE + EN), danach
  `scripts/xcstrings.py format`.
- **Keine rechte Buchungsspalte** in dieser Ansicht, solange sie ihre eigene Vorschau hat.
- **Neue Filter auf dem Mac** als weitere Zeile in `filterRows` (Beschriftung über `filterRow` und in der Liste
  `labels`, die die Spaltenbreite bestimmt), nicht mit eigener Beschriftung im Picker: sonst beginnt das Feld an einer
  anderen Kante.
- **Farben in der Zeile:** jede Farbe über `DocumentRow.tint`, damit sie auf der blauen Auswahl hell wird.

## Prüfen

```bash
cd GiroCrabApp
swift build -q && swift test -q --filter DocumentFiltersTests
.build/debug/GiroCrabMac --demo-seed --view belege --screenshot /tmp/belege.png
.build/debug/GiroCrabMac --demo-seed --view belege --window-width 760 --text-size larger --screenshot /tmp/belege-schmal.png
# gewählte Zeile: --pick wählt das Dokument, dessen Titel den Text enthält
.build/debug/GiroCrabMac --demo-seed --view belege --pick hotel --light --screenshot /tmp/belege-auswahl.png
```

Die Aufnahme zeigt die Auswahl ohne Fokus (grau); die blaue Auswahl mit Fokus nur in der laufenden App nach einem Klick
in die Liste. `--pick` wirkt nur mit `--demo-seed` und einmal pro Start.

iPhone: App mit `--demo-seed --view belege` im Simulator starten. Die Demo hat zwölf Dokumente; die jüngeren sind
ungelesen, drei ältere zeigen die Typ- und Jahresfilter.
