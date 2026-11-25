# Ablage von Bankbelegen beim Import

Der Bankabruf speichert den Typ eines Belegs, sein Ablagekonto und gegebenenfalls seine
Buchungsverknüpfung. Eine Wertpapierabrechnung kann am Depot liegen und zugleich bei
der Geldbuchung auf dem Verrechnungskonto erscheinen.

- Trade Republic verwendet Dokumenttitel und Abrufereignis. Depotauszüge und Wertpapierabrechnungen
  gehören ans Depot. Kontoauszüge und Zinsbelege gehören ans Verrechnungskonto. Eine mitgelieferte
  Buchungskennung verknüpft den Beleg auch über diese beiden Konten hinweg.
- comdirect verwendet Titel, IBAN oder Kontonummer aus dem PDF. REST und FinTS mit PostBox
  verwenden dafür dieselbe Zuordnung. Ohne eindeutigen Hinweis dient das einzige Girokonto
  als Ablage. Bei mehreren Girokonten bleibt das Ablagekonto offen.
- FNZ und Quirion verwenden die gemeinsamen Titelregeln für Dokumenttypen. Ihre Ablage
  folgt weiterhin dem Bankabruf.

Titelregeln und Prüfbeispiele stehen in `girocrab-core/src/documents.rs`. Bei mehreren Treffern
gilt die erste Regel. Die Konto- und Depotregeln stehen beim jeweiligen Connector. Die
PDF-Textauswertung läuft lokal.

## Buchungsverknüpfung anhand des Beleginhalts

Bei Trade Republic und comdirect versucht der Core beim Abruf auch Belege ohne
Buchungskennung zu verknüpfen. Das funktioniert in beiden Reihenfolgen: Ein später
abgerufener Beleg findet eine bereits gespeicherte Buchung und umgekehrt. Der
Abgleich berücksichtigt neue Dokumente, Dokumente mit erstmals erkanntem
Ablagekonto und neu abgerufene Buchungen. Die Datumsangaben im PDF bestimmen,
welche Buchungen in Betracht kommen, auch wenn das Erstellungsdatum fehlt
oder deutlich abweicht. Bereits gespeicherte Buchungen zählen bei der Auswahl
weiterhin mit. Ein erneuter Abruf derselben Daten stellt eine manuell gelöste
Verknüpfung bei unverändertem Ablagekonto nicht wieder her. Eine neue, besser
passende Buchung kann weiterhin mit dem Beleg verknüpft werden. Der Abgleich
erhält das Ablagekonto und bestehende Verknüpfungen.

Für eine Verknüpfung müssen der Betrag, der Gegenstand und ein Datum im PDF zur
Buchung passen. Der Abgleich berücksichtigt nur gebuchte Umsätze. Bei mehreren
Treffern entscheidet der kleinste Datumsabstand bis höchstens fünf Tage. Ein
Gleichstand bleibt auch bei schwächeren Hinweisen auf das Wertpapier offen. Hat
der beste Treffer bereits einen Beleg, bleibt das neue PDF offen.

- Trade Republic verwendet den Wertpapiernamen aus dem Titel und einen eindeutigen
  Betrag im PDF. Als Buchungskonto kommt das Verrechnungskonto infrage.
- Bei comdirect (REST und FinTS mit PostBox) zählen Abrechnungsbetrag, Buchungsrichtung
  und ISIN, WKN oder Wertpapiername. Die Währung folgt dem Abrechnungsbetrag.
  Bei mehreren Girokonten muss der Beleg das Buchungskonto eindeutig nennen,
  etwa mit einer IBAN oder einer als Verrechnungskonto bezeichneten Kontonummer.
  Kontoauszüge, Steuerunterlagen und reine Hinweise haben keine einzelne Buchung.

Die bankabhängigen Regeln stehen in `girocrab-core/src/connectors/document_matching.rs`
und beim Trade-Republic-Connector. Die Tests prüfen synthetische PDFs, beide
Abrufreihenfolgen und mehrdeutige Fälle. Für eine weitere Bank liefert der
Connector die fachlichen Kriterien, der Core verwendet denselben Abgleich.

## Vorhandene Bankbelege neu einsortieren

In den Einstellungen des Bankzugangs lässt sich „Dokumente neu einsortieren …“
für Trade Republic, comdirect (REST und FinTS mit PostBox), FNZ und Quirion
auslösen. GiroCrab prüft gespeicherte Bankbelege lokal. Es können sich
Dokumenttypen und auch manuell gewählte Ablagekonten ändern.
Bestehende Buchungsverknüpfungen bleiben erhalten. Bei Trade Republic und
comdirect können offene Belege neu verknüpft werden, auch nach einem früheren
manuellen Lösen der Verknüpfung.

comdirect-PostBox-Belege ohne Ablagekonto werden nur übernommen, wenn im
Tresor genau ein comdirect-Zugang aufgezeichnet ist. Ein früherer, gelöschter Zugang
zählt mit. FNZ und Quirion übernehmen die gemeinsamen Titelregeln für
PostBox-Dokumente.

Zum fortsetzbaren Abruf der Belege und Buchungen siehe [Trade Republic](trade-republic-connector.md).
