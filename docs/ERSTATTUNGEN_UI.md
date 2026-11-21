# Erstattungen verknüpfen (UI-Aufbau)

Diese Seite beschreibt, wie eine Erstattung mit der ursprünglichen Buchung verknüpft wird (#714): die Einträge im
Kontextmenü der Buchungsliste, das Blatt zum Zuordnen (`RefundPickerSheet`), das Zeichen in der Zeile
(`RefundLinkMarker`) und die Karte „Erstattung“ im Inspector (`RefundLinksCard`), alle in `RefundsUI.swift`.

## Grundprinzip

Die Verknüpfung dokumentiert nur, was zusammengehört. **Beträge, Kategorien, Splits und Kontostände ändern sich
nicht**; die Verrechnung in den Kategorien bleibt Sache der Kategorisierung. Der Core (`refunds.rs`) hält je
Erstattung genau eine ursprüngliche Buchung; eine ursprüngliche Buchung kann mehrere (Teil-)Erstattungen haben.

- Eine Erstattung ist ein Eingang (Betrag > 0), die ursprüngliche Buchung ein Ausgang (Betrag < 0). Buchungen mit
  Betrag 0 haben keine Einträge.
- Keine Ketten: Eine Erstattung ist nie zugleich ursprüngliche Buchung einer anderen.
- Konten und Währungen dürfen sich unterscheiden (Kauf mit der Karte, Erstattung aufs Girokonto).
- Eine Splitbuchung wird als ganze Buchung verknüpft; ihre Teile bleiben, wie sie sind.
- Wird eine der beiden Buchungen gelöscht, verschwindet die Verknüpfung aus der Anzeige.
- Verknüpfen und Aufheben erreichen über iCloud die anderen Geräte (neueste Änderung gewinnt) und lassen sich mit
  Bearbeiten › Widerrufen zurücknehmen.

## Kontextmenü der Buchungsliste

Unter „Umsatz aufteilen (Splitbuchung) …“:

| Buchung | Eintrag |
| --- | --- |
| Eingang ohne Verknüpfung | „Ursprüngliche Buchung zuordnen …“ |
| Eingang mit Verknüpfung | „Ursprüngliche Buchung ändern …“ und „Verknüpfung zur ursprünglichen Buchung aufheben“ |
| Ausgang ohne Erstattung | „Erstattung zuordnen …“ |
| Ausgang mit Erstattungen | „Erstattungen bearbeiten …“ |

## Blatt zum Zuordnen

Oben die Buchung, von der aus geöffnet wurde (Name, Datum, Konto, Betrag), ein Satz zur Auswahl, der Hinweis, dass
sich nichts an Beträgen und Kategorien ändert, und die Suche (Name oder Verwendungszweck, gefiltert im Core).

Darunter zuerst die **verknüpften** Buchungen (✓, „Aufheben“), bei einer ursprünglichen Buchung mit der Summe
„Erstattet: … von …“, wenn alle Erstattungen ihre Währung haben (verschiedene Währungen werden nie summiert, #569).
Dann bis zu drei **Vorschläge** des Core (`refund_suggestions`), jeder mit seinem Grund, aufgefüllt in dieser
Reihenfolge und darin nach Nähe zum Datum: gleicher Betrag in der Gegenrichtung (gleiche Währung), gleiche Gegenpartei
oder gleicher Händler innerhalb von zwei Monaten, gleiche Referenz (Bestell- oder Rechnungsnummer im Verwendungszweck,
End-to-End-Referenz).

Darunter die **weiteren Buchungen** der Gegenrichtung aus allen Konten, nächstes Datum zuerst: für eine Erstattung
aus den 365 Tagen davor, für eine ursprüngliche Buchung aus den 365 Tagen danach und nur Eingänge, die noch keiner
anderen Buchung zugeordnet sind. Mit einem Suchbegriff entfällt die Grenze von 365 Tagen, die Liste zeigt dann alle
Treffer.

„Zuordnen“ (oder Doppelklick) verknüpft sofort, das Blatt bleibt offen: So lassen sich mehrere Teilerstattungen
nacheinander zuordnen. Bei einer Erstattung ersetzt eine neue Wahl die bisherige. „Fertig“ oder Esc schließt.

Die Vorschläge werden abseits des Hauptthreads gelesen (`vaultOffMain`): Der Core bestimmt für jede Buchung des
Zeitraums den Händler. Nur das Ergebnis der jüngsten Suche wird gezeigt.

## Zeichen in der Zeile

Hinter dem Namen: ↩ bei einer verknüpften Erstattung, ↩ im Kreis bei einer Buchung mit Erstattungen, jeweils mit
Tooltip und Bedienungshilfen-Text. Der Core liefert das Zeichen mit jeder Zeile (`Booking.refund_link`); nach dem
Verknüpfen ändert die App nur die betroffenen Zeilen, die Liste wird nicht neu geladen.

## Karte „Erstattung“ im Inspector

Nur bei verknüpften Buchungen sichtbar, in der Standardreihenfolge nach „Tags“ (im Bearbeitungsmodus als Platzhalter).
Eine Erstattung zeigt die ursprüngliche Buchung, eine ursprüngliche Buchung ihre Erstattungen und die Summe wie im
Blatt. Ein Klick auf eine Zeile wählt diese Buchung in der Liste aus (`revealBooking`: weitet Zeitraum und Filter,
wenn sie dort nicht steht), ✕ hebt die Verknüpfung auf, „Ändern …“ / „Bearbeiten …“ öffnet das Blatt.

## Persönliche Daten

Namen, Konto und Verwendungszweck tragen `.sensitive()`, Beträge und die Summe `.sensitive(.amounts)`, das Suchfeld
`.sensitive()`. Das Blatt wird mit `.capturableSheet` gezeigt.

## Prüfen

`swift run GiroCrabMac --demo-seed --demo-sheet refund` öffnet das Blatt für den ersten Eingang der Demo-Daten,
`--demo-sheet refund-linked` mit bereits verknüpfter Buchung (zeigt auch Zeichen und Karte).

## Geteilte Buchungen

Die Verknüpfung gilt der ganzen Buchung, nicht einem Teil: Eine geteilte ursprüngliche Buchung behält ihre Teile und
Kategorien, „Erstattet: … von …“ rechnet mit ihrem Gesamtbetrag. Teilen, neu teilen oder die Teilung aufheben lässt
die Verknüpfungen bestehen.

## Auswertung

Eine verknüpfte Erstattung zählt nicht als Einnahme. Wie eine Erstattung, die der Core am Buchungstext erkennt,
mindert sie die Ausgaben: in den Kennzahlen der Buchungsliste, im Cashflow und in den Auswertungen, die auf derselben
Einordnung beruhen. Nach dem Aufheben der Verknüpfung gilt wieder die Einordnung des Core. Beträge, Kategorien und
Kontostände ändern sich dadurch nicht.
