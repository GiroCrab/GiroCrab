# Erstattungen zuordnen

Gemeinsames Verhalten der Oberflächen unter macOS, Linux und Windows.

Die Kernregeln und ihre Begründung stehen in
[ADR 0002](adr/0002-refund-links-and-projection.md).

## Einstieg und Aufbau

Unter macOS und Linux öffnet das Kontextmenü einer Buchung „Erstattung zuordnen …“.
Unter Windows öffnet „Erstattung …“ im Inspector die Zuordnung. Das funktioniert
für Eingänge, Ausgaben und aufgeteilte Buchungen mit einer Gesamtsumme von null.
Die Buchungsliste kennzeichnet zugeordnete Erstattungen mit einem Rückpfeil.
Bei einer teilweisen Zuordnung steht der zugeordnete Betrag daneben; bei
Splitbuchungen wird der zugeordnete Betrag ebenfalls angezeigt.

Das Blatt zeigt die eigene Buchung, bei Splitbuchungen eine Teileauswahl,
die verknüpften Buchungen in Zuteilungsreihenfolge, bis zu drei Vorschläge
und eine durchsuchbare Liste weiterer Buchungen. Vorschläge nennen den Grund:
gleicher Betrag, gleicher Händler oder gleiche Referenz. Bereits zugeordnete
Gegenstellen werden nicht erneut angeboten. Ausgeschöpfte Zeilen sind nicht
auswählbar. Eine leere Suche wird ausdrücklich angezeigt.

## Teile und Richtung

Die Auswahl trägt Kategorie, Notiz und Betrag. Ihre Identität ist die stabile
UUID des Teils, nie seine Position. Ein Teil mit einem Betrag ungleich
null ist vorausgewählt. Ein positiver Teil zahlt zurück, ein negativer Teil
wird zurückgezahlt. Das gilt auch für gemischte Splitbuchungen. Ganze Buchungen
können nur dann Endpunkte sein, wenn sie nicht aufgeteilt sind.

## Verknüpfung und Rückgängig

Unter macOS zeigt die Inspector-Karte „Erstattungen“ beide Rollen und alle aktiven
Verknüpfungen der ausgewählten Buchung. Das Kreuz entfernt die einzelne
Verknüpfung. „Bearbeiten …“ öffnet das Blatt. Verknüpfte Zeilen verwenden nur
die Bezeichnungen, zugeordneten Beträge und offenen Beträge der Verknüpfung;
Gegenbuchungen werden nicht zusätzlich geladen.

Die Zuordnung verändert weder Bankbuchungen noch Splits oder Kontostände.
Der zugeordnete Anteil wird in den Auswertungen am Datum der Erstattung in der
Kategorie des ursprünglichen Teils berücksichtigt. Zuteilung und Kapazitäten
berechnet ausschließlich der Kern.

Das Kreuz entfernt genau eine Verknüpfung. „Rückgängig“ im Blatt und der
Widerrufen-Befehl verwenden das vom Kern gelieferte Lösch-Token. Eine neuere
Änderung kann damit nicht überschrieben werden; die ursprüngliche Position
der Verknüpfung bleibt erhalten. Widerrufen gilt nur im selben Tresor.

## Laden und Datenschutz

### macOS

Alle neuen Kernabfragen und Schreibvorgänge laufen außerhalb des UI-Threads.
Die eigene Buchung wird auf ihr Konto und ihren Buchungstag eingegrenzt gelesen.
Buchung, Teile, Verknüpfungen und Vorschläge werden nach Tresor, Buchung,
Teil-UUID und Revision geladen. Nur die verzögerte Kandidatensuche hängt zusätzlich
vom Suchtext ab. Suchänderungen laden ausschließlich Kandidaten nach; Datumsangaben
dieser Zeilen verwenden dieselbe Formatierung wie die Buchungsliste.
Beim Wechsel eines Teils sind alte Zeilen sofort nicht mehr bedienbar.
Nach Änderungen und Synchronisation werden offene Daten neu gelesen; ein
Tresorwechsel schließt das Blatt. Gelöschte Buchungen schließen es ebenfalls.

Das Blatt wird über die Berichtserfassung präsentiert. Namen und Notizen sind
persönlich, Kategorien sind Beschriftungen, Beträge sind Betragsdaten. Gemischte
Teilebeschriftungen decken alle drei Datenschutzstufen ab. Das Layout verwendet
skalierte Schrift und lässt Namen umbrechen, ohne Beträge zu verkleinern.

## Reproduzierbare Ansichten

### macOS

Nur im Demo-Tresor erzeugen `--demo-sheet refund-split`, `refund-negative` und
`refund-linked` fiktive Buchungen und Teilverknüpfungen. Die ersten beiden
öffnen das Blatt, die dritte Variante lässt die markierte Buchungsliste offen.
`--text-size larger` und `--redact` prüfen Schriftgröße und Berichtsschutz.

### Linux

Das Kontextmenü eines Umsatzes öffnet die Erstattungszuordnung. Bei aufgeteilten
Umsätzen wählt die Teileauswahl den eigenen Buchungsteil. Positive Teile werden
ursprünglichen Buchungen zugeordnet; negative Teile erhalten Erstattungen.
Aktive Verknüpfungen stehen in Zuteilungsreihenfolge vor bis zu drei Vorschlägen
und der durchsuchbaren Buchungsliste. Zugeordnete Beträge stehen in der Umsatzzeile.

Lesen und Schreiben laufen im Hintergrund. Suchanfragen und Ergebnisse sind an
Anfragegeneration und Tresor gebunden. Ein Tresorwechsel schließt die Ansicht.
Änderungen laden Umsätze, Kategorien, Kreisdiagramm, Cockpit und Planung neu.
Datenschutzmasken schützen Namen, Beträge und Kategorien getrennt, auch in der
geöffneten Teileauswahl. Banktexte werden als einfacher Text angezeigt.
Aufheben und tokenbasiertes Rückgängig ändern keine Benutzersplits.

Im Demo-Tresor zeigen `--demo-sheet refund`, `refund-reverse`, `refund-before`
und `refund-linked` fiktive Zuordnungen. `--privacy=all` prüft den Datenschutz.

### Windows

Die fokussierbare Schaltfläche „Erstattung …“ im Inspector öffnet die Zuordnung,
auch für Ausgaben und Splitbuchungen mit Gesamtsumme null. Die Umsatzzeile zeigt
Rückpfeil und zugeordneten Betrag. Die Teileauswahl verwendet UUIDs; Verknüpfungen
bleiben in der vom Kern gelieferten Reihenfolge. Vorschläge und Suchergebnisse
stehen in getrennten Abschnitten. Fokussierbare Schaltflächen erlauben Zuordnen,
Aufheben und tokenbasiertes Rückgängig auch per Tastatur. Escape schließt den Dialog.

Lesen und Schreiben laufen auf Hintergrundthreads. Anfragezähler und
Tresoridentität verwerfen alte Ergebnisse. Während Teilwechseln und Änderungen
sind alte Zeilen nicht bedienbar. Nach Synchronisation, Split-, Kategorie- und
Linkänderungen werden Buchungen, Kategorien, Analysen und Cockpit neu geladen.
Beim Aktivieren des Datenschutzes verschwinden alte Dialoginhalte sofort;
neu geladene Daten verwenden den aktuellen Datenschutzstatus.

„Werte ausblenden“ aktiviert den Datenschutz auch im geöffneten Windows-Dialog;
„Werte anzeigen“ beendet ihn wieder.

`GIROCRAB_LANGUAGE=en` wählt englische Texte der Erstattungsansicht (Standard:
Deutsch). Im Demo-Modus (`GIROCRAB_DEMO_SEED=1`) erzeugt
`GIROCRAB_REFUND_DEMO=split`, `reverse` oder `linked` reproduzierbare Ansichten.
