# Dokumente den Buchungen zuordnen

Bankdokumente (Abrechnungen, Kontoauszüge, Steuerbescheinigungen, Mitteilungen) liegen als PDF im Tresor
(`booking_documents`). Ein Dokument gehört zu einem **Konto** und, wenn es einen Geldfluss belegt, zusätzlich zu
einer **Buchung**. Dieses Dokument beschreibt das Konzept; welche Titel, Konten und Quellen es gibt, weiß jeder
Connector selbst.

## Zuordnung

Dokumentkonto und Buchungsverknüpfung sind unabhängig: Der Adapter bestimmt Typ und fachliches Konto
(z. B. Wertpapierabrechnung ans Depot); `booking_id` verweist bei einem Einzelbeleg zusätzlich auf die
Geldbuchung. Details und Ausnahmen stehen unter „Dokumentkonto und Buchungskonto“.

Eine passende Quellkennung (`RemoteDocument.booking_external_id` → `external_id`) verknüpft direkt.
Fehlt sie, sucht `connectors/document_matching.rs` über den PDF-Text: Betrag, Datum (±`slack_days`),
Gegenstand (Name, ISIN oder WKN), gegebenenfalls Richtung und Kontokennung müssen zur Buchung passen.
Nur ein eindeutiger nächster Treffer zählt. Zeitraum- und Informationsdokumente erhalten keine Einzelbuchung.

## Was ein Connector beisteuert

`DocumentMatchRules` kapselt das Bankwissen; der Suchablauf bleibt gemeinsam:

| Felder | Bedeutung |
|---|---|
| `booking_account` | zulässige Buchungskonten anhand von `external_ref` und Kontoart |
| `subjects`, `subjects_from_text` | Gegenstand aus Titel und gegebenenfalls PDF-Text |
| `link_types` | Dokumenttypen, die einen einzelnen Geldfluss belegen dürfen |
| `inflow`, `direction_required` | Richtung und Pflicht, sie eindeutig zu erkennen |
| `account_from_text` | Buchungskonto anhand von IBAN oder Kontonummer eingrenzen |
| `slack_days` | erlaubter Datumsabstand |
| `amount_labels` | Kennzeichnung des Abrechnungsbetrags; ohne solche Labels muss der Text genau einen unterschiedlichen Betrag liefern |
| `retype`, `account_for` | Typ und Dokumentkonto beim erneuten Einsortieren |
| `orphan_source_prefix` | Herkunftshinweis für Dokumente ohne Konto und Buchung; Übernahme nur bei einem einzigen aufgezeichneten Zugang der Bank |

Neue Banken brauchen lesbare PDFs und Buchungstexte mit einem passenden Gegenstand. Scans ohne Textebene
bleiben unverknüpft. Gleiche Beträge, Daten und Gegenstände können mehrdeutig sein; dann bleibt die Zuordnung offen.

## Wann der Abgleich läuft

* **Nach jedem Abruf** für den Datumsbereich der gerade gespeicherten Buchungen (`persist::store_fetch`), damit
  nachgeladene Buchungen ihre schon vorhandenen Dokumente finden.
* **Auf Wunsch für alle Dokumente:** Bei Trade Republic löst „Dokumente neu einsortieren“ ihn über alle
  Dokumente aus und sortiert zugleich Typ und Konto anhand der Titel neu.

## Gemeinsamer Ablauf und Bankwissen

Gemeinsam (`connectors/document_matching.rs`): die Reihenfolge Typ → Konto → Buchung, die Kandidatensuche samt
Richtung (`match_by_amounts`), die eindeutige Verknüpfung und das erneute Einsortieren gespeicherter Dokumente
(`reprocess_documents`, dieselben Regeln wie beim Import, wiederholbar ohne Änderung). Beim Zugang bleibt: die
Regeln (`DocumentMatchRules`: Buchungskonten, Gegenstand, Richtung, Typ und Zielkonto je Dokument) und die Auslegung der
Metadaten der Bank. `reprocess_rules_for` und `rules_for_connection` nennen, wer mitmacht: Trade Republic und
comdirect (REST und FinTS mit PostBox) mit Inhaltsabgleich, FNZ und Quirion nur mit Typ (`PORTAL_STATEMENTS`).
Trade Republic, FNZ, Quirion und comdirect verwenden denselben Ablauf für „Dokumente neu einsortieren“.

### Regeln ergänzen

Titelregeln stehen in `girocrab-core/data/document_rules.json`. Ergänze für eine neue Dokumentart ein
Prüfbeispiel. Regeln für Konto, Richtung und die Buchungsverknüpfung gehören zum jeweiligen Adapter und
brauchen Tests mit anonymisierten Beispieldokumenten und Buchungen.

## Dokumentkonto und Buchungskonto

`booking_documents.account_id` ist die **fachliche Zuordnung** eines Dokuments, `booking_id` die Buchung, die es belegt. Beides
bleibt getrennt: Eine Wertpapierabrechnung oder ein Dividendenbeleg gehört zum **Depot** und belegt eine Geldbuchung auf dem
**Verrechnungskonto**. Das Konto hängt am Dokument, nicht am Weg, auf dem es ankam: Ein Beleg aus den Buchungsdetails und
derselbe Beleg aus der Aktivitätsliste (später über den Inhalt verknüpft) liegen am selben Konto.

| Dokument | Dokumentkonto | Buchung |
|---|---|---|
| Wertpapierkauf/-verkauf, Sparplan, Dividendenabrechnung | Depot | passende Geldbuchung auf dem Verrechnungskonto |
| „Dividende Wahlweise“, Kapitalmaßnahmen, Informationen zu einem Wertpapier | Depot | keine (kein Geldfluss) |
| Kontoauszug, Zinsen, Gebühren des Verrechnungskontos | Verrechnungskonto | nur bei einem passenden Einzelbeleg |
| Depotauszug | Depot | keine Einzelbuchung |
| Steuerbescheinigung, allgemeine Informationen | wie abgelegt | keine |

Neue Belege werden beim Import einsortiert. Beim nächsten Abruf bleibt die Ablage vorhandener Belege
bestehen. „Dokumente neu einsortieren“ wendet die Regeln nach Bestätigung auf die Belege des gewählten
Bankzugangs an. Dabei können auch manuell gewählte Ablagekonten geändert werden. Bestehende
Buchungsverknüpfungen bleiben erhalten. Der Inhaltsabgleich überspringt einen Treffer, der
bereits einen Beleg hat. Die Dokumentliste filtert nach dem Ablagekonto.
In der Buchungsansicht erscheint derselbe Beleg an seiner Buchung.

## Reihenfolge der Ankunft

Der Abgleich hängt nicht davon ab, dass Dokument und Buchung im selben Abruf kommen. `store_fetch` sucht in einem
Zeitfenster, das die Buchungen und die Dokumente des Abrufs umfasst: Kommt die Buchung später, findet sie das
gespeicherte Dokument; kommt das PDF später, findet es eine ältere gespeicherte Buchung. Ein Dokument, das schon eine
Buchung hat, wird nicht noch einmal angefasst, ein wiederholter Abruf legt kein zweites Dokument an
(`source_url` ist eindeutig). Gesucht wird nur im Bankzugang des Dokuments und in den Konten, die die Regeln des Zugangs
nennen (Trade Republic: `tr:cash`, comdirect: die Girokonten, nie das Depot). Portal-Importe (FNZ, Quirion, EquatePlus) speichern über eigene Funktionen und laufen nicht durch `store_fetch`:
Sie haben keine Inhaltsregeln und verknüpfen deshalb nichts.

## Angebundene Zugänge und Dokumentarten

| Zugang | Abrufweg | Dokumentarten | Konto | Buchung |
|---|---|---|---|---|
| Trade Republic | `timelineDetailV2` | Abrechnung, Beleg, Kontoauszug, Steuer, PostBox | `tr:cash` oder `tr:depot` aus der Quelle | Kennung, sonst Inhalt (Betrag, Datum, Gegenstand, Richtung) |
| comdirect | REST-PostBox und FinTS mit PostBox | Abrechnung (Kauf, Verkauf, Ertrag), Kontoauszug (Finanzreport), Steuer, PostBox (gemeinsame Typisierung `account_document_type`) | IBAN oder Konto-/Depotnummer im PDF-Text, Wertpapierdokumente am Depot; ohne eindeutigen Hinweis nur das einzige Girokonto, sonst kein Konto | Abrechnungen und Ertragsgutschriften über Inhalt: Betrag, Datum, Richtung, Währung und ISIN, WKN oder Name gegen Buchungen des Girokontos; Auszüge und Steuerpapiere nie |
| FNZ (finvesto) | Portal, eigener Import | Konto- und Depotauszüge (`account_document_type`) | Konto oder Depot aus den Metadaten des Dokuments | nie: Auszüge belegen keine Einzelbuchung |
| Quirion | Portal, eigener Import | wie FNZ | wie FNZ | nie |
| DKB | FinTS (`HKEKA`, bei DKB `HKEKP`) | Kontoauszug | Konto aus dem Abruf | nie |

Offen bleiben bewusst: Auszüge, Steuerbescheinigungen und andere Perioden- oder Informationsdokumente
(`link_types` und Typen ohne Gegenstand), mehrdeutige Treffer, Dokumente ohne lesbaren Text, Buchungen anderer
Bankzugänge und Dokumente einer anderen Richtung (Kauf gegen Gutschrift). Steht ein Betrag neben einer
Währung, muss diese zur Buchung passen; fehlt eine Währungsangabe am Betrag, schließt der Abgleich ihn nicht aus.

**comdirect** liefert in den PostBox-Metadaten nur Name, Datum und Kennung; belegt ist, was in den PDFs steht (Analysebericht
über lokal geladene Dokumente): Wertpapierabrechnungen nennen „Wertpapierkauf“ oder „Wertpapierverkauf“, die Depotnummer
und ISIN und WKN; die IBAN steht meist erst weiter unten im Text; Finanzreports sind Periodenauszüge. Daraus folgt:
- *Typ* aus dem Titel (`type_from_title`, gemeinsam mit allen Portalen), dazu `finanzreport` (Kontoauszug) und
  `ertragsgutschrift`/`thesaurierend` (Abrechnung) in `document_rules.json`.
- *Konto* über `assign_account` (IBAN oder Nummer im Text, Wertpapierdokumente ans Depot). Ohne eindeutigen Hinweis
  bleibt ein Dokument **ohne Konto**, außer das Login hat genau ein Girokonto (`default_account`); das erste von
  mehreren Girokonten ist keine Evidenz.
- *Buchung* über `document_matching::COMDIRECT`. Eignung: nur `abrechnung`, mit bekannter Richtung (Kauf/Verkauf/Ertrag);
  Kosteninformationen, Mitteilungen, Finanzreports und Steuerpapiere bekommen nie eine Einzelbuchung, auch wenn ihr Text
  ISIN, Beträge und Daten nennt (`link_types`, `direction_required`). Kandidaten: Buchungen der Girokonten; nennt das PDF
  ein Verrechnungskonto (IBAN, sonst Nummer), zählen nur dessen Buchungen, nennt es keines und gibt es mehrere
  Girokonten, bleibt das Dokument offen (`account_from_text`). Der Abgleich bevorzugt „Ausmachender Betrag“.
  Fehlt diese Angabe, kommen „Zu Ihren Lasten/Gunsten vor Steuern“ infrage. Kurs und Provision sind keine
  Abrechnungsbeträge. Eine Währung am passenden Betrag muss mit der Buchung übereinstimmen
  (`amounts_with_currency`); eine fehlende Währungsangabe verhindert die Verknüpfung nicht. Dazu Datum
  (±5 Tage), Richtung und ISIN, WKN oder Name in Verwendungszweck oder Gegenpartei. Das Dokument behält das Depot als
  Konto, die Buchung liegt auf dem Girokonto. Mehrdeutiges bleibt offen.
  Ein Test mit echten Buchungen steht aus. Er muss zeigen, ob der Verwendungszweck die Wertpapierkennung
  enthält und wann ein Betrag „vor Steuern“ der tatsächlichen Geldbuchung entspricht.
- *Bestehende Dokumente:* „Dokumente neu einsortieren“ nutzt dieselbe Kontoermittlung wie der Import (`account_for_stored`) und
  verschiebt Dokumente, die auf keinem Konto oder auf dem ersten Girokonto (dem früheren Rückfall) liegen, dorthin, wohin
  der Text zeigt. Auch eine von Hand gewählte Ablage kann sich ändern. Dokumente ganz ohne Konto und Buchung (Quelle
  `comdirect:postbox:…`) werden nur erfasst, wenn der Tresor genau einen aufgezeichneten comdirect-Zugang hat (einschließlich gelöschter Zugänge); sonst ist die Herkunft nicht sicher.
- Das gilt für die REST-PostBox (`ConnectorKind::Comdirect`) und für FinTS mit PostBox (`FinTsGeneric` mit comdirect-BLZ;
  `rules_for_connection` erkennt es am Connector-Schlüssel).

**DKB:** Zu trennen sind lokal vorhandene und abrufbare Dokumentarten. Im lokalen Ordner liegen neben Kontoauszügen
Wertpapierabrechnungen, Ertragsabrechnungen, Depotauszüge (Jahresbrief), Kosteninformationen und Depotüberträge mit
Depot-, Kunden- und Auftragsnummer, ISIN und WKN im Text. Abrufbar ist über FinTS (Voreinstellung `dkb`, BLZ 12030000)
nur der Kontoauszug (`kontoauszug` am Konto, `fints:hkekp:<Konto>:<Hash>`; HKEKP-Zeiträume sind nicht verlässlich, der
Inhalt dient als Schlüssel). Für die übrigen Arten gibt es keinen Abrufweg; die Regeln in `document_rules.json` (Titel
„Wertpapier Abrechnung Kauf“) sind nur für die Analyse und für einen künftigen Abruf vorbereitet.

EquatePlus lädt optional seine Bibliothek, hat aber noch keinen Inhaltsabgleich. Bei weiteren Zugängen
(FinTS, bunq, easybank, Fin4U) hängt die Verknüpfung vom jeweiligen Abrufweg ab.
