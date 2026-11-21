# Erstattungen verknüpfen (UI-Aufbau)

Diese Seite beschreibt, wie eine Erstattung mit der ursprünglichen Buchung verknüpft wird (#714): die Einträge im
Kontextmenü der Buchungsliste, das Blatt zum Zuordnen (`RefundPickerSheet`), das Zeichen in der Zeile
(`RefundLinkMarker`) und die Karte „Erstattung“ im Inspector (`RefundLinksCard`), alle in `RefundsUI.swift`.

## Grundprinzip

Die Verknüpfung dokumentiert nur, was zusammengehört. **Beträge, Kategorien, Splits und Kontostände ändern sich
nicht**. In den Auswertungen nach Kategorie zählt der zugeordnete Teil einer Erstattung in der Kategorie der Buchung
(bei geteilten Buchungen: des gewählten Teils), die er zurückzahlt; der nicht zugeordnete Rest bleibt in der Kategorie
der Erstattung. Der Core (`refunds.rs`) hält Paare aus
Erstattung und ursprünglicher Buchung: Eine ursprüngliche Buchung kann mehrere (Teil-)Erstattungen haben, eine
Erstattung kann mehrere Buchungen zurückzahlen (eine Sammelerstattung).

**Was zählt:** Eine Buchung gilt höchstens bis zu ihrem Betrag als erstattet, und jede Erstattung zählt nur einmal.
Der Core verteilt dazu jede Erstattung (älteste zuerst) auf ihre ursprünglichen Buchungen (älteste zuerst); was eine
Buchung nicht mehr aufnimmt, bleibt für die nächste. `RefundLinks.refunded_cents` (für die ursprüngliche Buchung) und
`assigned_cents` (für die Erstattung) tragen das Ergebnis; bei verschiedenen Währungen fehlen sie (#569).

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
| Eingang mit Verknüpfung | „Ursprüngliche Buchungen bearbeiten …“ und „Verknüpfung zur ursprünglichen Buchung aufheben“ |
| Ausgang ohne Erstattung | „Erstattung zuordnen …“ |
| Ausgang mit Erstattungen | „Erstattungen bearbeiten …“ |

## Blatt zum Zuordnen

Oben die Buchung, von der aus geöffnet wurde (Name, Datum, Konto, Betrag), ein Satz zur Auswahl, der Hinweis, dass
sich nichts an Beträgen und Kategorien ändert, und die Suche (Name oder Verwendungszweck, gefiltert im Core).

Darunter zuerst die **verknüpften** Buchungen (✓) mit der Summe, wenn alle Beteiligten dieselbe Währung haben
(verschiedene Währungen werden nie summiert, #569):

- ursprüngliche Buchung: „Erstattet: … von …“, nie mehr als ihr Betrag. Sind die Erstattungen zusammen höher, sagt
  eine zweite Zeile, wie viel sie betragen und dass der Rest weiteren Buchungen zugeordnet werden kann.
- Erstattung: „Zugeordnet: … von …“ und, solange etwas übrig ist, „Noch offen: …“.

Jede verknüpfte Zeile zeigt unter ihrem Betrag „Zugeordnet: …“ (`RefundBooking.share_cents`): den Teil der Erstattung,
der auf diese Verknüpfung entfällt – höchstens der Betrag der Buchung und höchstens der Rest der Erstattung. Ist die
Erstattung schon aufgebraucht, steht dort 0 in der Warnfarbe (nur noch bei Verknüpfungen von einem anderen Gerät). Dasselbe zeigt die Karte im Inspektor.

Dann bis zu drei **Vorschläge** des Core (`refund_suggestions`), jeder mit seinem Grund, aufgefüllt in dieser
Reihenfolge und darin nach Nähe zum Datum: gleicher Betrag in der Gegenrichtung (gleiche Währung), gleiche Gegenpartei
oder gleicher Händler innerhalb von zwei Monaten, gleiche Referenz (Bestell- oder Rechnungsnummer im Verwendungszweck,
End-to-End-Referenz).

Darunter die **weiteren Buchungen** der Gegenrichtung aus allen Konten, nächstes Datum zuerst: für eine Erstattung
aus den 365 Tagen davor, für eine ursprüngliche Buchung aus den 365 Tagen danach. Mit einem Suchbegriff entfällt die Grenze von 365 Tagen, die Liste zeigt dann alle
Treffer.

Jede Zeile ist als Ganzes eine Schaltfläche, der Kreis davor eingeschlossen: Ein Klick verknüpft sofort, ein Klick
auf eine verknüpfte Zeile (✓) hebt genau diese Verknüpfung auf. Das Blatt bleibt offen: So lassen sich mehrere
Teilerstattungen oder mehrere ursprüngliche Buchungen nacheinander zuordnen. „Fertig“ oder Esc schließt.

Die Vorschläge werden abseits des Hauptthreads gelesen (`vaultOffMain`): Der Core bestimmt für jede Buchung des
Zeitraums den Händler. Nur das Ergebnis der jüngsten Suche wird gezeigt.

## Zeichen in der Zeile

Hinter dem Namen: ↩ bei einer verknüpften Erstattung, ↩ im Kreis bei einer Buchung mit Erstattungen, jeweils mit
Tooltip und Bedienungshilfen-Text. Der Core liefert das Zeichen mit jeder Zeile (`Booking.refund_link`); nach dem
Verknüpfen ändert die App nur die betroffenen Zeilen, die Liste wird nicht neu geladen.

## Karte „Erstattung“ im Inspector

Nur bei verknüpften Buchungen sichtbar, in der Standardreihenfolge nach „Tags“ (im Bearbeitungsmodus als Platzhalter).
Eine Erstattung zeigt ihre ursprünglichen Buchungen, eine ursprüngliche Buchung ihre Erstattungen und die Summe wie im
Blatt. Ein Klick auf eine Zeile wählt diese Buchung in der Liste aus (`revealBooking`: weitet Zeitraum und Filter,
wenn sie dort nicht steht), ✕ hebt die Verknüpfung auf, „Bearbeiten …“ öffnet das Blatt.

## Persönliche Daten

Namen, Konto und Verwendungszweck tragen `.sensitive()`, Beträge und die Summe `.sensitive(.amounts)`, das Suchfeld
`.sensitive()`. Das Blatt wird mit `.capturableSheet` gezeigt.

## Prüfen

`swift run GiroCrabMac --demo-seed --demo-sheet refund` öffnet das Blatt für den ersten Eingang der Demo-Daten,
`--demo-sheet refund-linked` mit bereits verknüpfter Buchung (zeigt auch Zeichen und Karte).
`--demo-sheet refund-split` teilt den ersten Vorschlag in zwei eigene Kategorien: das Blatt führt statt der
geteilten Buchung ihre Teile („Teil 1 · Garten“), die Verknüpfung gilt dem gewählten Teil. `--demo-sheet refund-split-out`
öffnet das Blatt dieser geteilten Ausgabe; „Erstattung für“ wählt dort den Teil, dem die nächste Erstattung gilt.

## Geteilte Buchungen

Die Verknüpfung gilt der ganzen Buchung, nicht einem Teil: Eine geteilte ursprüngliche Buchung behält ihre Teile und
Kategorien, „Erstattet: … von …“ rechnet mit ihrem Gesamtbetrag. Teilen, neu teilen oder die Teilung aufheben lässt
die Verknüpfungen bestehen.

## Auswertung

Eine verknüpfte Erstattung zählt nicht als Einnahme, soweit sie Buchungen zugeordnet ist. Wie eine Erstattung, die der
Core am Buchungstext erkennt, mindert dieser Teil die Ausgaben: in den Kennzahlen der Buchungsliste, im Cashflow und in
den Auswertungen, die auf derselben Einordnung beruhen. Der noch nicht zugeordnete Rest bleibt Einnahme
(`Booking.refund_income_cents`, `models::flow_parts`), bis auch er zugeordnet ist.

In der Buchungszeile steht neben dem Erstattungssymbol nur dann ein Betrag (`Booking.refund_part_cents`), wenn die
Buchung erst zum Teil erstattet bzw. zugeordnet ist; ganz erstattete oder zugeordnete Buchungen zeigen nur das Symbol. Nach dem Aufheben der Verknüpfung gilt wieder die Einordnung des Core. Beträge, Kategorien und
Kontostände ändern sich dadurch nicht.

## Reihenfolge und Grenze

Die Anteile werden in der Reihenfolge gefüllt, in der die Verknüpfungen angelegt wurden (`refund_links.changed_ms`):
Die zuerst verknüpfte Buchung bekommt ihren Anteil zuerst, die nächste höchstens den Rest. So stehen die verknüpften
Zeilen auch in der Auswahl und im Inspektor. Ist eine Erstattung vollständig zugeordnet oder eine Buchung vollständig
erstattet (`open_cents == 0`), nimmt sie keine weitere Verknüpfung an: Der Core lehnt sie ab, die Auswahl zeigt die
übrigen Zeilen abgeblendet mit Hinweis, und Vorschläge lassen solche Buchungen aus.
