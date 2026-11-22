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
Der Core füllt dazu die Verknüpfungen in der Reihenfolge, in der sie angelegt wurden (siehe „Reihenfolge und Grenze“);
was eine Buchung nicht mehr aufnimmt, bleibt für die nächste. `RefundLinks.refunded_cents` (für die ursprüngliche Buchung) und
`assigned_cents` (für die Erstattung) tragen das Ergebnis; bei verschiedenen Währungen fehlen sie (#569).

- Eine Erstattung ist ein eingehender Betrag (> 0), die ursprüngliche Buchung ein ausgehender (< 0). Das gilt je
  Betrag, nicht je Buchung: Auch ein eingehender Teil einer Ausgabe zahlt zurück (ein Guthaben, mit dem ein Teil eines
  Kaufs bezahlt wurde), und ein ausgehender Teil einer Einnahme kann erstattet werden (eine abgezogene Gebühr).
  Buchungen mit Betrag 0 haben keine Einträge.
- Keine Ketten: Ein Betrag ist nie zugleich Erstattung und ursprüngliche Buchung. Eine geteilte Buchung mit Teilen
  beider Richtungen kann beides sein, mit verschiedenen Teilen; ihre eigenen Teile lassen sich nicht miteinander
  verknüpfen.
- Konten und Währungen dürfen sich unterscheiden (Kauf mit der Karte, Erstattung aufs Girokonto).
- Bei einer Splitbuchung gilt die Verknüpfung einem ihrer Teile (siehe „Geteilte Buchungen“); ihre Teile bleiben, wie
  sie sind. Das gilt für beide Seiten: bei einer geteilten Ausgabe für den Teil, der erstattet wird, bei einer
  geteilten Einnahme für den Teil, der die Erstattung ist.
- Wechselt eine verknüpfte Buchung die Seite (eine von Hand geführte Ausgabe wird zur Einnahme oder umgekehrt), ist
  sie keine ursprüngliche Buchung bzw. Erstattung mehr: Ihre Verknüpfungen werden aufgehoben.
  Ändert die Bank den Betrag einer abgerufenen Buchung so, dass sie die Seite wechselt, bleibt die Verknüpfung
  gespeichert, zählt aber nichts mehr; sie lässt sich weiterhin aufheben. Wurde bei einer der beiden Buchungen zuvor
  Datum oder Betrag von Hand bearbeitet, werden solche Verknüpfungen beim Abgleich zwischen den Geräten aufgehoben.
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
aus den 365 Tagen davor, für eine ursprüngliche Buchung aus den 365 Tagen danach. Mit einem Suchbegriff entfallen die Grenze von 365 Tagen und die
Richtung: Die Liste zeigt dann alle Treffer, auch Buchungen nach der Erstattung bzw. Eingänge vor der Buchung (eine
Erstattung kann vor dem Kauf gebucht sein). Die Vorschläge bleiben bei der Richtung.

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
`--demo-sheet refund-split-in` teilt die Einnahme selbst; „Erstattung aus“ wählt den Teil, der die Erstattung ist.
`--demo-sheet refund-credit` öffnet das Blatt eines Kaufs, der zum Teil mit einem Guthaben bezahlt wurde.

## Geteilte Buchungen

Eine geteilte ursprüngliche Buchung behält ihre Teile und Kategorien. Die Verknüpfung nennt den Teil, den die
Erstattung zurückzahlt (`refund_links.original_part`, ab 1), und nimmt höchstens dessen Betrag auf; „Erstattet: … von …“
rechnet mit dem Gesamtbetrag der Buchung.

Ist die **Einnahme** geteilt (etwa Gehalt mit erstatteten Auslagen), nennt die Verknüpfung den Teil, der die
Erstattung ist (`refund_links.refund_part`, ab 1): Nur dieser Teil zahlt zurück, höchstens mit seinem Betrag, und nur
er zählt nicht mehr als Einnahme; die übrigen Teile bleiben in ihren Kategorien, wie sie sind. Im Blatt der Einnahme
wählt „Erstattung aus“ den Teil für die nächste Verknüpfung, im Blatt einer Ausgabe steht eine geteilte Einnahme mit
ihren eingehenden Teilen zur Auswahl. Eine Verknüpfung der ganzen geteilten Einnahme (`refund_part` 0: aus der Zeit vor
der Teilung, oder von einer Oberfläche ohne diese Auswahl über `link_refund`) nimmt ihren Anteil aus den Teilen vom
letzten zum ersten.

**Teile der Gegenrichtung.** Die Rolle hängt am Teil: Ein eingehender Teil einer Ausgabe (Guthaben) wird über
`refund_part` verknüpft und zahlt eine andere Buchung zurück; ein ausgehender Teil einer Einnahme (Gebühr) wird über
`original_part` verknüpft und kann erstattet werden. Im Blatt wählt dieselbe Auswahl den Teil; je nach Richtung des
gewählten Teils sammelt das Blatt ursprüngliche Buchungen oder Erstattungen (`refund_part_candidates` mit
`pays_back`). In der Auswahl einer Erstattung stehen deshalb auch die ausgehenden Teile von Einnahmen, in der Auswahl
einer Ausgabe die eingehenden Teile anderer Ausgaben. In der Buchungsliste trägt eine Buchung, die etwas zurückzahlt,
das Zeichen der Erstattung, auch wenn sie selbst eine Ausgabe ist. In der Auswertung senkt nur, was eine Ausgabe
zurückzahlt, die Ausgaben; was eine Gebühr in einer Einnahme zurückzahlt, bleibt Einnahme. Ein Guthaben-Teil ändert
die Kennzahlen nicht (das Geld der Buchung bleibt dasselbe), er zählt aber in der Kategorie der zurückgezahlten
Buchung statt in seiner eigenen. Hat eine Buchung nach dem Ändern oder Aufheben ihrer Aufteilung keinen Betrag der
verknüpften Richtung mehr, werden diese Verknüpfungen aufgehoben.

Oberflächen ohne Auswahl des zahlenden Teils (`refund_candidates`, `link_refund`) bekommen eine Einnahme nur als
Ganzes angeboten und keine Guthaben-Teile; die ausgehenden Teile, auch die von Einnahmen, stehen dort zur Auswahl.

Die Teile werden über ihre Position gezählt. Deshalb übernimmt der Abgleich zwischen Geräten die Teile in ihrer
Reihenfolge (`sync_merge::bookings::load_splits`), statt sie zu sortieren.

Wird die Aufteilung einer Ausgabe oder einer Einnahme geändert, folgen die Verknüpfungen (`refunds::split_changed`,
aus `planning::store_splits`):

- **Aufteilung aufgehoben:** Die Verknüpfungen gelten der ganzen Buchung. Jede Erstattung wird bis zu dem aufgefüllt,
  was die Buchung noch offen hat, nicht mehr nur bis zum Betrag ihres früheren Teils.
- **Anders aufgeteilt** (andere Beträge oder andere Anzahl der Teile): Die Verknüpfung bleibt bei ihrem Teil, solange
  die Buchung an dieser Position noch einen Teil derselben Richtung hat; sie gilt dann seinem neuen Betrag und nie den
  anderen Teilen. Gibt es den Teil nicht mehr (weniger Teile, oder der Teil hat die Richtung gewechselt), gilt die
  Verknüpfung der ganzen Buchung. Ihr Anteil wird dann vom letzten Teil zum ersten verteilt, jeder Teil wird gefüllt,
  bis der Anteil aufgebraucht ist; so zählt er in den Kategorien dieser Teile. Dasselbe gilt für eine als Ganzes
  verknüpfte Buchung, die erst danach geteilt wird, und für eine Verknüpfung von einem anderen Gerät, deren Teil es
  hier nicht gibt.
- **Nur Kategorie oder Notiz eines Teils geändert:** Die Verknüpfung bleibt bei ihrem Teil.

Die umgestellten Verknüpfungen behalten ihren Platz unter den Verknüpfungen ihrer Erstattung (der Stempel des
Anlegens `created_ms` bleibt; nur `changed_ms` ist neu, damit die Änderung die anderen Geräte erreicht). Eine neue Verknüpfung zu einer geteilten Buchung nennt
weiterhin einen Teil.

## Auswertung

Eine verknüpfte Erstattung zählt nicht als Einnahme, soweit sie Buchungen zugeordnet ist. Wie eine Erstattung, die der
Core am Buchungstext erkennt, mindert dieser Teil die Ausgaben: in den Kennzahlen der Buchungsliste, im Cashflow und in
den Auswertungen, die auf derselben Einordnung beruhen. Der noch nicht zugeordnete Rest bleibt Einnahme
(`Booking.refund_income_cents`, `models::flow_parts`), bis auch er zugeordnet ist. Eine Verknüpfung zwischen
verschiedenen Währungen ordnet nichts zu (#569): Die Erstattung bleibt dort ganz Einnahme.

Das Kreisdiagramm der Ausgaben im Kopf (`AccountExpensePie`, `Vault::list_category_expenses`) zeigt die Ausgaben je
Kategorie abzüglich des zugeordneten Teils; er zählt am Datum und auf dem Konto der Erstattung. Die Summen der
Seitenleiste folgen dagegen wie die Buchungsliste dem Filter „Ausgaben“ / „Einnahmen“ (#621): dort trägt die Zeile der
Erstattung die Minderung, unter „Ausgaben“ stehen also die Buchungen ohne Abzug.

Zwei Folgen davon wirken ohne Erklärung wie ein Fehler, deshalb nennt die Karte „Erstattung“ im Inspektor sie in
einer Zeile unter den Summen, sobald etwas zugeordnet ist: Die Minderung zählt im Monat der Erstattung, der Monat des
Kaufs zeigt die Ausgabe weiter in voller Höhe; und unter dem Filter „Einnahmen“ steht der zugeordnete Betrag als
geminderte Ausgabe (positiver Wert bei „Ausgaben“), nicht als Einnahme.

In der Buchungszeile steht neben dem Erstattungssymbol nur dann ein Betrag (`Booking.refund_part_cents`), wenn die
Buchung erst zum Teil erstattet bzw. zugeordnet ist; ganz erstattete oder zugeordnete Buchungen zeigen nur das Symbol. Nach dem Aufheben der Verknüpfung gilt wieder die Einordnung des Core. Beträge, Kategorien und
Kontostände ändern sich dadurch nicht.

## Reihenfolge und Grenze

Die Anteile werden in der Reihenfolge gefüllt, in der die Verknüpfungen angelegt wurden (`refund_links.created_ms`;
bei gleichem Stempel von zwei Geräten nach Datum und Bankkennung der Buchungen, auf beiden Geräten gleich):
Die zuerst verknüpfte Buchung bekommt ihren Anteil zuerst, die nächste höchstens den Rest. So stehen die verknüpften
Zeilen auch in der Auswahl und im Inspektor. Ist eine Erstattung vollständig zugeordnet oder eine Buchung vollständig
erstattet (`open_cents == 0`), nimmt sie keine weitere Verknüpfung an: Der Core lehnt sie ab, die Auswahl zeigt die
übrigen Zeilen abgeblendet mit Hinweis, und Vorschläge lassen solche Buchungen aus.
