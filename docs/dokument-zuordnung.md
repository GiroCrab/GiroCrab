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

Titelregeln und Prüfbeispiele stehen in `girocrab-core/data/document_rules.json`. Die Reihenfolge
der Regeln entscheidet bei mehreren Treffern. Konto- und Depotregeln bleiben beim jeweiligen
Connector. Die PDF-Textauswertung läuft lokal.
