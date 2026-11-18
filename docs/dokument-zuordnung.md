# Dokumente den Buchungen zuordnen

Bankdokumente (Abrechnungen, Kontoauszüge, Steuerbescheinigungen, Mitteilungen) liegen als PDF im Tresor
(`booking_documents`). Ein Dokument gehört zu einem **Konto** und, wenn es einen Geldfluss belegt, zusätzlich zu
einer **Buchung**. Dieses Dokument beschreibt das Konzept; welche Titel, Konten und Quellen es gibt, weiß jeder
Connector selbst (z. B. [Trade Republic](trade-republic-connector.md)).

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
* **Auf Wunsch für alle Dokumente:** Bei Trade Republic löst „Historie und Zuordnung neu aufbauen“ ihn über alle
  Dokumente aus und sortiert zugleich Typ und Konto anhand der Titel neu.

## Gemeinsamer Ablauf und Bankwissen

Gemeinsam (`connectors/document_matching.rs`): die Reihenfolge Typ → Konto → Buchung, die Kandidatensuche samt
Richtung (`match_by_amounts`), die eindeutige Verknüpfung und das erneute Einsortieren gespeicherter Dokumente
(`reprocess_documents`, dieselben Regeln wie beim Import, wiederholbar ohne Änderung). Beim Zugang bleibt: die
Regeln (`DocumentMatchRules`: Buchungskonten, Gegenstand, Richtung, Typ und Zielkonto je Dokument) und die Auslegung der
Metadaten der Bank. `reprocess_rules_for` und `rules_for_connection` nennen, wer mitmacht: Trade Republic und
comdirect (REST und FinTS mit PostBox) mit Inhaltsabgleich, FNZ und Quirion nur mit Typ (`PORTAL_STATEMENTS`).
„Historie und Zuordnung neu aufbauen“ (Trade Republic) und „Dokumente neu einsortieren“ (FNZ, Quirion, comdirect) rufen denselben Ablauf.

### Regeldatei und Analyse-Skript (eine Regel oder Bank ergänzen)

**Wer nutzt welchen Teil von `girocrab-core/data/document_rules.json`?**

| Abschnitt | Analyse-Skript | Core |
|---|---|---|
| `dokumenttypen`, `standard` | nein | ja: `documents::type_from_title` (Typ aus dem Titel) |
| `arten` (mit `typ`) | ja: feinere Berichtsarten | nein; `typ` nennt die Abbildung auf den App-Typ |
| `banken` (`erkennung`, eigene `arten`) | ja: Bank erkennen, bankeigene Arten zuerst | nein |
| `pruefbeispiele` | ja: prüft `art` | ja: prüft `typ` (`account_document_type`) |

Die Analyse unterscheidet feiner (Kauf, Verkauf, Sparplan, Dividende, Depotauszug, Finanzreport …), die App kennt fünf Typen
(`abrechnung`, `kontoauszug`, `steuer`, `postbox`, `beleg`). Die Abbildung steht je Art im Feld `typ`; verbindlich ist sie nur,
soweit die `pruefbeispiele` sie belegen. Die Richtung (Kauf/Verkauf) braucht die App nicht als Typ, sondern für den Abgleich
(`document_matching::inflow_in`, Code).

**Wann genügt eine JSON-Regel, wann braucht es Code?**
- JSON genügt, wenn ein Wort im Titel den Typ entscheidet (neue Dokumentfamilie, neues Schlüsselwort, bankeigene Art für die
  Analyse).
- Code (Adapter) braucht, was die Quelle oder der PDF-Aufbau entscheidet: welches Konto oder Depot (Metadaten der Quelle,
  `assign_account`), der Gegenstand eines Dokuments (`subjects` in `DocumentMatchRules`), welche Konten Buchungen tragen
  (`booking_account`), die Richtung, wie weit der Textkopf gelesen wird. Verlässliche Quellmetadaten haben Vorrang vor Titel
  und PDF-Text. Dateinamen aus einem lokalen Export sind nie Voraussetzung: der Core sieht nur Titel, Metadaten und Bytes.

**Eine Bank oder Dokumentfamilie ergänzen**
1. Analyse am eigenen Ordner ausführen (unten), im Bericht „Art auch ohne Dateiname aus dem Text erkannt“ lesen.
2. Titelwörter in `dokumenttypen` oder `arten` eintragen; Vorrang: der erste Treffer gewinnt, spezifische Arten stehen
   vor allgemeinen (`Verkauf` vor `Kauf`, weil „Verkauf“ „kauf“ enthält), bankeigene Arten gelten vor den allgemeinen.
3. Bank unter `banken` mit `erkennung` anlegen; bankeigene `arten` nur, wo das allgemeine Muster fehlgreift.
4. Pro Familie ein erfundenes `pruefbeispiel` mit `art` und `typ` eintragen (Titel nach dem Muster, nie echte Namen,
   Nummern oder Beträge). Fehlklassifizierungen verhindert man hier: ein Beispiel pro Familie und eines für jeden bekannten
   Stolperstein („Wertpapierverkauf“ ist kein Kauf, „Kosteninformation“ ist keine Abrechnung).
5. Braucht die Bank Konto-, Gegenstands- oder Richtungslogik: Adapter und `DocumentMatchRules` (Beispiele:
   `trade_republic::DOCUMENT_MATCH`, `document_matching::COMDIRECT`); die Regeln greifen beim Import und beim erneuten
   Einsortieren (`reprocess_documents`) gleich.

**Lokale Dokumentanalyse**

```
python3 scripts/analyse-dokumente.py "/Pfad/zum/Ordner" --name "Vorname Nachname" --bank comdirect --out bericht.md
```

Das Skript liest nur, nutzt kein Netzwerk und schwärzt IBAN, Namen, Adressblock, Mail, Telefon und lange Zahlen. Es schreibt
nichts, wenn im Bericht noch eine IBAN, Mailadresse, einer der angegebenen Namen oder ein Name neben dem Platzhalter steht.
Alle Namen im Haushalt mit `--name` angeben; den Bericht vor dem Weitergeben selbst durchlesen.

**Tests**
- `python3 -m unittest scripts/test_analyse_dokumente.py`: Regeldatei, Erkennung, Schwärzung, gemeinsame Beispiele.
- Core, Typregeln: `cargo test -q -p girocrab-core --lib documents::tests`.
- Core, Abgleich: `cargo test -q -p girocrab-core --lib document_matching` und
  `cargo test -q -p girocrab-core --lib comdirect` (Reihenfolge der Ankunft, Isolation, Währung/Richtung, Periodendokumente,
  manuelle Zuordnung, Kontowahl).

**Was die Analyse belegt und was erst ein Test**
Der Bericht zeigt, woran sich ein Dokument erkennen lässt (Titel, Text, Kopf, Kennungen). Er beweist nicht, dass die App es
richtig ablegt: Das Konto belegt erst ein Test von `assign_account` beziehungsweise des Adapters, die Buchungsverknüpfung erst ein
Test mit Buchung und PDF durch `store_fetch` (Integrationstest in `connectors/tests/mod.rs`). Ob die echten Buchungstexte der
Bank den Gegenstand nennen, zeigt nur ein Abruf mit echten Daten.

**Kleines Beispiel einer neuen Regel**
Eine Bank „Beispielbank“ nennt ihre Wertpapierabrechnungen „Orderbestätigung Kauf …“ (Titel, nicht in den Regeln).
In `document_rules.json`:
```
{ "name": "Beispielbank", "erkennung": ["beispielbank"],
  "arten": [ { "name": "Kauf", "typ": "abrechnung", "muster": "orderbestätigung kauf" } ] }
```
und in `dokumenttypen` bei `abrechnung` das Wort `orderbestätigung`; dazu das Beispiel
`{ "bank": "Beispielbank", "titel": "Orderbestätigung Kauf 5 St. WKN A1B2C3", "art": "Kauf", "typ": "abrechnung" }`.
Erwartet: die Analyse führt solche Dokumente als „Beispielbank · Kauf“ (vorher „Sonstiges“); die App legt sie als
`abrechnung` ab (vorher `postbox`, weil kein Wort passte). Welches Konto und welche Buchung, entscheidet weiter der Adapter.

### Eine neue Bank anbinden: der Ablauf

Die Erfahrung mit comdirect, DKB, Quirion und Trade Republic: Am meisten Zeit kostete nicht die Regel, sondern herauszufinden,
**was die App überhaupt sieht**. In dieser Reihenfolge geht es schneller:

1. **Export aus GiroCrab.** Dokumente über den Konnektor abrufen (nicht aus einem anderen Programm: dessen Dateinamen und
   Zeiträume sind nicht die der App) und in der Belegansicht je Konto in einen leeren Ordner exportieren.
2. **Analyse mit dem Text der App.** `cargo build -p girocrab-core --example pdf_text`, dann
   `python3 scripts/analyse-dokumente.py <Ordner> --name "…" --extractor "$PWD/target/debug/examples/pdf_text" --etikett "<Etikett>"`.
   Ohne `--extractor` liest das Skript mit `pdftotext`, und was ein PDF-Betrachter lesen kann, kann der Text-Auszug der App noch lange nicht.
3. **Befund dem Ort im Code zuordnen:**

| Befund im Bericht | Ort |
|---|---|
| Text leer oder Zeichensalat | `documents::extract_pdf_text` (bisher: EBCDIC-Text eines Großrechner-Drucks, Hex-Strings, ein „/Image“ im Nachbarobjekt) |
| Art aus dem Titel falsch | `document_rules.json` (`dokumenttypen`, `arten`, ein `pruefbeispiel`) |
| Konto oder Depot unklar | Adapter: `assign_account` (comdirect), `document_account_ref` (Trade Republic), Metadaten der Quelle haben Vorrang |
| Welcher Betrag belegt die Buchung | `DocumentMatchRules::amount_labels` (Etikett im Text, auch als Spaltenüberschrift mit den Werten in der Zeile darunter) |
| Wie nennt die Buchung das Wertpapier | `DocumentMatchRules::subjects` (ISIN, WKN, Name); **im Verwendungszweck der Umsatzliste nachsehen, nicht im PDF** |
| Kauf, Verkauf, Ertrag | `document_matching::inflow_in` |
| Es fehlen alte Dokumente oder Umsätze | Abrufzeitraum („Tage beim ersten Abruf“, 90), PostBox-Seitenlimit (500 Dokumente), Zeitfenster des Abrufs |

4. **Einbauen, mit Tests:** Prüfbeispiele in der Regeldatei (Skript und Core prüfen dieselben Titel), ein Integrationstest mit
   Buchung und PDF durch `store_fetch` in beiden Ankunftsreihenfolgen (`connectors/tests/mod.rs`, Muster: die `cd_*`-Tests).
5. **Gegenprobe im eigenen Tresor** (Sicherung vorher): Abrufen, „Dokumente neu einsortieren“; mit `GIROCRAB_DEPOT_DEBUG=1`
   schreibt der Abgleich eine Zeile `document matching: …` mit Zählern, warum Dokumente offen bleiben (Typ nicht geeignet, kein
   lesbarer Text, kein abgerechneter Betrag, Richtung unbekannt, kein Konto genannt, kein Kandidat, nicht eindeutig). Das
   Abrufprotokoll hält das Ergebnis des Neu-Einsortierens fest.
6. **Weitergeben:** nur Zähler und geschwärzte Zeilen. Das Skript schreibt nichts, wenn noch ein Name oder eine IBAN im Bericht steht.

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

Trade Republic entscheidet das an einer Stelle (`document_account_ref`: ein Wort wie „Dividende“ allein genügt nicht, es zählt
die Dokumentfamilie), beim Import und beim erneuten Einsortieren. „Historie und Zuordnung neu aufbauen“ (Trade Republic)
verschiebt auch bereits verknüpfte Belege ans Depot; Buchungsverknüpfung, PDF, Quellkennung und Gelesenstatus bleiben.
Die automatische Verknüpfung (`link_unlinked_by_content`) und das manuelle Verknüpfen (`link_document`) setzen nur `booking_id`;
ein Dokument ohne Konto bekommt dabei das Konto der Buchung. Das Dokumentkonto ändert nur die ausdrückliche Aktion
„Konto zuordnen“ (`set_document_account`): sie behält eine bestehende Verknüpfung, merkt sich die Wahl (`account_manual`) und
wird beim erneuten Einsortieren nicht überschrieben. Die Dokumentliste filtert nach dem Dokumentkonto (ohne eines nach dem der
Buchung); in der Buchungsansicht erscheint ein Beleg an seiner Buchung, ohne zweite Kopie.

*Einschränkung:* Was Nutzer vor Schema 32 von Hand verschoben haben, ist nicht von dem zu unterscheiden, was der Import
gewählt hat (die Herkunft wurde nicht gespeichert). Das Neu-Einsortieren kann solche Dokumente wieder an das Konto der Regel
legen. Der Abgleich zwischen Geräten übernimmt `account_manual` nicht.

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
| Trade Republic | `timelineDetailV2`, `timelineActivityLog` | Abrechnung, Beleg, Kontoauszug, Steuer, PostBox | `tr:cash` oder `tr:depot` aus der Quelle | Kennung, sonst Inhalt (Betrag, Datum, Gegenstand, Richtung) |
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
  Girokonten, bleibt das Dokument offen (`account_from_text`). Betrag: nur der abgerechnete Betrag nach „Ausmachender
  Betrag“, nicht Kurs, Provision oder Steuer (`amount_labels`; ohne dieses Etikett kein Treffer). Währung: Steht eine Währung am passenden Betrag, muss sie mit der Buchung übereinstimmen
  (`amounts_with_currency`); eine fehlende Währungsangabe verhindert die Verknüpfung nicht. Dazu Datum
  (±5 Tage), Richtung und ISIN, WKN oder Name in Verwendungszweck oder Gegenpartei. Das Dokument behält das Depot als
  Konto, die Buchung liegt auf dem Girokonto. Mehrdeutiges bleibt offen.
  *Nicht belegt:* ob die Buchungstexte der Bank ISIN, WKN oder Namen enthalten und ob jede Abrechnung „Ausmachender Betrag“
  trägt (aus anderen Banken bekannt, bei comdirect nur angenommen). Fehlt etwas davon, bleibt das Dokument unverknüpft
  (keine Falschzuordnung); ein Test mit echten Buchungen steht aus.
- *Bestehende Dokumente:* „Dokumente neu einsortieren“ nutzt dieselbe Kontoermittlung wie der Import (`account_for_stored`) und
  verschiebt Dokumente, die auf keinem Konto oder auf dem ersten Girokonto (dem früheren Rückfall) liegen, dorthin, wohin
  der Text zeigt. Was der Nutzer woanders abgelegt hat, bleibt. Dokumente ganz ohne Konto und Buchung (Quelle
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
