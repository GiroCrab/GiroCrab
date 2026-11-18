# Trade-Republic-Anbindung (Web-Login + WebSocket)

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zur Trade Republic. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen der Bank verstoßen.
> This connector is an independent community project and is not affiliated with Trade Republic. Interface: unofficial. Using unofficial interfaces may violate the bank's terms of service.
<!-- sync:connector-note:end -->

Status: **eingeschränkt** (`BANK_MATRIX`: inoffizielle Schnittstelle) – `ConnectorKind::TradeRepublic`,
Implementierung in `girocrab-core/src/connectors/trade_republic.rs`. Die Erstanmeldung nutzt wegen
des WAF-Schutzes das Browserfenster; spätere Abrufe verwenden die gespeicherte Sitzung.

Trade Republic veröffentlicht keine API. Der Connector folgt dem beobachtbaren Verhalten der
Web-App (wie das Open-Source-Projekt `pytr`). Er kann brechen, sobald Trade Republic die Web-App
ändert. Unten stehen nur Pfade, Feldnamen und Regeln, keine echten Werte.

## Zugang

* **Login:** Mobilnummer (wird auf E.164 normalisiert, z. B. `0171 …` → `+49171…`) und App-PIN.
* **Zwei-Faktor:** Code aus der App oder per SMS. Die Sitzungs-Cookies (`tr_session`, `tr_refresh`)
  liegen verschlüsselt im Verbindungsstatus; spätere Abrufe brauchen keinen neuen Code, solange
  die Sitzung erneuert werden kann. Cookies aus einem Browser-Login lassen sich über
  `Vault::set_connection_session` übergeben.

## Ablauf

| Schritt | Anfrage | Auswertung |
|---|---|---|
| 1 | `POST https://api.traderepublic.com/api/v1/auth/web/login` (Fallback `v2`) | Telefonnummer und PIN → `processId`. |
| 2 | `POST …/login/{processId}/{code}` (optional vorher `…/sms`) | Antwort oft leer, die Sitzung kommt als `Set-Cookie`. |
| 3 | `GET …/auth/web/session` | Erneuert eine gespeicherte Sitzung. |
| 4 | `WSS wss://api.traderepublic.com` (`connect 31 {…}`) | Alle weiteren Daten über Subscriptions (`sub <id> {…}`), Antworten `A`, Fehler `E`, Ende `C`. |

Subscriptions im WebSocket:

| Typ | Zweck |
|---|---|
| `cash` (Fallback `availableCash`) | Verrechnungskonto-Saldo (`tr:cash`) |
| `timelineTransactions` | Buchungen mit Geldbewegung, seitenweise über `after` |
| `timelineActivityLog` | „Aktivitäten“: alle Ereignisse, auch ohne Geldbewegung (Dokumente, siehe unten) |
| `timelineDetailV2` | Detailansicht je Eintrag, enthält den Abschnitt „Dokumente“ mit den PDF-Links |
| `accountPairs` | Depotnummer der Sitzung (Standardprodukt zuerst) |
| `compactPortfolioByType` (Fallback `compactPortfolio`) | Depotpositionen |
| `instrument`, `ticker` | Name, Typ, Fonds-Kategorie und Kurs je Position |

## Konten

Zwei Konten je Verbindung:

* `tr:cash` – Verrechnungskonto mit Buchungen und Saldo.
* `tr:depot` – Wertpapierdepot mit Positionen (Capability „Wertpapiere“).

## Depotpositionen und Klassifizierung

* Stückzahl, durchschnittlicher Einstandskurs und Kurs kommen aus Portfolio und `ticker`;
  der Wert wird in Cent gerundet (halb weg von null).
* Eine Antwort ohne bekanntes Layout leert das Depot nicht: sie sagt nichts über die Positionen.
* **Wertpapiergeschäfte (Aktivitäten):** Sie entstehen aus den Detailansichten der Trade-Zeilen
  (Stückzahl, Preis, Gebühr, Steuer). Aktuelle Trades werden im Sync-Fenster gelesen, ältere in Etappen
  (siehe „Historie in Etappen“). Aktivitäten werden über die Trade-ID aktualisiert.
* **Klassifizierung (#351):** Der Connector übernimmt nur, was Trade Republic ausdrücklich
  meldet, als importierte Werte (`ClassificationValues`), getrennt nach Anlageklasse und Instrument.
  Der Instrumenttyp entscheidet, nie ein Namensbestandteil („Netflix“ ist kein ETF).

  | Meldung von Trade Republic | Anlageklasse | Instrument |
  |---|---|---|
  | Krypto | Krypto | Direktanlage |
  | Anleihe | Anleihe | Einzelwert |
  | Derivat, Optionsschein, Knock-out, Faktor | offen | Derivat |
  | Name enthält ETC / ETN als ganzes Wort | Basiswert, bei ETC sonst Rohstoff | ETC/ETN |
  | `etf`, oder `fund` mit Kategorie `etf` | Basiswert des Fonds | ETF |
  | `fund` | Basiswert des Fonds | Fonds |
  | `stock` | Aktien | Einzelwert |

  Die Anlageklasse eines Fonds folgt aus `fundInfo.underlyingClass` („Aktien“, „Festverzinslich“,
  Geldmarkt, Misch-, Rohstofffonds). Was offen bleibt, klären die ISIN- und Namensregeln im Kern
  (`asset_classification.rs`); eigene Zuordnungen des Nutzers haben immer Vorrang.

## Dokumente (PDF-Download)

Beleg-Abruf in zwei Quellen, danach je Eintrag `timelineDetailV2` → Abschnitt „Dokumente“ → PDF:

* **`timelineTransactions`:** Belege zu Käufen, Verkäufen, Sparplänen, Dividenden, Zinsen. Sie hängen
  über `booking_external_id` an der jeweiligen Buchung.
* **`timelineActivityLog`:** Dokumente ohne Geldbewegung – Konto- und Depotauszüge, Quartalsabschlüsse,
  Steuerbescheinigungen, Ertragsaufstellungen, Kapitalmaßnahmen. Sie gehören zum ganzen Konto und
  werden ohne Buchung (`booking_id = NULL`) im Dokumentenarchiv von `tr:cash` bzw. `tr:depot` abgelegt.
  Ist der Endpunkt nicht verfügbar, läuft der Abruf ohne diese Dokumente weiter.

Dokumenttyp nach dem Titel (`document_type`, geprüft an den Titeln eines echten Exports mit gut 2.000
Trade-Republic-PDFs):

| Titel enthält | Typ | Beispiele |
|---|---|---|
| `steuer`, `ertragsaufstellung`, `vorabpauschale` | `steuer` | Steuerbescheinigung, Vorabpauschale |
| `auszug`, `statement`, `abschluss` | `kontoauszug` | Kontoauszug, Depotauszug, Cryptoauszug |
| `wahlweise`, `information`, `kundenanschreiben`, `kundenvereinbarung`, `bedingungsänderung`, `vertragsunterlagen`, `mitteilung`, `löschbestätigung`, `prospekt`, `hauptversammlung`, `rückkaufangebot`, `geschäftsbedingungen`, `agb`, `agreement`, `datenschutz`, `leistungsverzeichnis`, `eignungsprüfung`, `treuhand`, `hinweise` | `postbox` | Dividende Wahlweise, Kosteninformation, Basisinformationsblatt, Prospekt, Löschbestätigung, Geschäftsbedingungen |
| `abrechnung`, `bestätigung`, `dividende`, `ausführungsanzeige` | `abrechnung` | Abrechnung, Abrechnung Ausführung, Ausführungsanzeige |
| sonst | `beleg` | |

Ältere Dokumente heißen bei Trade Republic nur „Dokumente“. Bei ihnen entscheidet der Ereignistext des
Zeitleisten-Eintrags: Dividende, Ausschüttung, Sparplan, Kauf oder Verkauf ergibt `abrechnung`, eine
Steuerbuchung `steuer`. Ein Beleg ohne Buchung wird `postbox`.

Das Dokumentkonto folgt der Dokumentfamilie (`document_account_ref`), unabhängig vom Abrufweg und einer
Buchungsverknüpfung: Wertpapierabrechnungen, Dividendenbelege, Depotauszüge und Wertpapiermitteilungen
gehören ans Depot; Kontoauszüge, Zinsen und Kontogebühren ans Geldkonto. Manuell gewählte Konten bleiben erhalten.

* **Duplikate:** Schlüssel ist die PDF-Adresse ohne den signierten Query-Teil; der UNIQUE-Index auf
  `source_url` verhindert doppelte Einträge.
* **Bereits geladene Einträge** merkt sich die Verbindung (`doc_items` im Status) und fragt sie nicht
  erneut ab. Je Sync werden bis zu 50 Dokumentkandidaten bearbeitet, während des historischen Abrufs bis zu 450;
  weitere bleiben für spätere Abrufe vorgemerkt. Ein Kandidat kann mehrere PDFs enthalten.

## Zuordnung zu Buchungen

Belege aus `timelineTransactions` werden über die Eintrags-ID mit ihrer Geldbuchung verknüpft.
Bei anderer ID (etwa aus dem Aktivitätsprotokoll) greift der gemeinsame
[Inhaltsabgleich](dokument-zuordnung.md) mit `trade_republic::DOCUMENT_MATCH`: Buchungen von `tr:cash`,
Datum ±5 Tage und Wertpapiername aus dem Titel. Da TR keine Labels für den Abrechnungsbetrag vorgibt,
muss das PDF genau einen unterschiedlichen Betrag liefern; Kurs und Gesamtbetrag im selben Text
verhindern eine automatische Verknüpfung. Ohne Treffer bleibt der Beleg am Dokumentkonto und ist manuell zuordenbar.

## Schalter „Dokumente abrufen“

Ist er beim Bankzugang ausgeschaltet, lädt der Abruf keine Beleg-Detailansichten (`doc_candidates` bleibt leer) und
merkt sich nichts von Belegen; die Wertpapier-Seite holt die Trade-Ansichten, die sie selbst braucht, weiter.
Wieder eingeschaltet, reicht das Fenster der Zeitleiste (und der Aktivitäten) bis zum Tag des Ausschaltens zurück
(`Documents::catchup_since()`, `window_start`), bis ein Abruf nichts Neues bringt und nichts auslassen musste
(`FetchResult::documents_incomplete`: fehlgeschlagene Detailansicht oder Download, mehr Belege als ein Abruf nimmt).

## Historie in Etappen und „Neu aufbauen“

Der Abruf liest die Wertpapier-Historie neueste zuerst in Etappen mit regulär 400 Trades (`HISTORY_BATCH`).
Der Grenztag wird vollständig gelesen; nötigenfalls kommt der nächste ältere Tag dazu, damit der
Datumszeiger auch bei mehr als 400 Trades an einem Tag vorankommt. In jeder
Etappe kommen die Wertpapiergeschäfte, die Buchungen des Verrechnungskontos zu diesen Trades (als „geprüft“
angelegt, damit „Neue Umsätze“ nicht überläuft) und die zugehörigen Belege. Bereits gespeicherte PDFs werden
nur verknüpft, nicht erneut geladen. Den Stand halten `trade_history_v3_before` und `trade_history_v3_complete`
im Status der Verbindung; solange die Historie nicht vollständig ist, zeigt die App einen Banner mit
„Weiter abrufen“.

Schlägt die Detailansicht eines Trades fehl, merkt sich der Zugang seine Kennung (`trade_history_v3_retry`)
und liest ihn im nächsten Abruf erneut, auch wenn der Historienzeiger schon weiter ist (dann wird die Zeitleiste bis
zum Ende gelesen); solange einer aussteht, gilt die Historie als nicht vollständig.

Unter Bankzugang › Status setzt **„Historie und Zuordnung neu aufbauen …“** den Stand zurück, sortiert die
gespeicherten Belege anhand ihrer Titel neu (Typ und Dokumentkonto, auch bei bereits verknüpften Belegen;
ein allgemeines „Dokumente“ behält seinen Typ, eine manuelle Kontowahl bleibt bestehen) und führt den inhaltlichen Abgleich über alle Belege aus.
Nichts wird erneut heruntergeladen; die folgenden Abrufe lesen die Historie erneut.

Offene Dokumentkandidaten (`doc_pending_v1`) und fehlgeschlagene Trades werden bis zum erfolgreichen
Abruf gespeichert; Warteschlangen werden nicht abgeschnitten. Auch Antworten ohne PDF (etwa eine
Anmeldeseite mit HTTP 200) gelten als fehlgeschlagen. Wird `timelineActivityLog` durch Fehler oder
Seitenlimit unterbrochen, halten `doc_listing_open_v1` und `doc_listing_cursor_v1` den Stand für den
nächsten Abruf fest. Offene Arbeiten halten den Banner „Weiter abrufen“ an (`history_incomplete`).

## Einschränkungen

* Der Zugang ist inoffiziell (wie die Web-App): Der Login läuft über das Browserfenster (WAF) mit 2FA; eine
  abgelaufene Sitzung erfordert eine neue Anmeldung. Ändert Trade Republic die Schnittstelle, kann der Abruf
  ausfallen.
* Nur lesender Zugriff: keine Überweisungen oder Aufträge.
* Wertpapier-Aktivitäten sind „best effort“ aus den Detailansichten; wie weit der Verlauf zurückreicht,
  bestimmt Trade Republic.
* PDFs werden nicht ausgewertet außer für den Abgleich oben; der Typ folgt dem Titel (bei „Dokumente“ dem
  Ereignistext).
* Positionen in Fremdwährung gehen nicht in die Summen ein; der Bewertungsverlauf entsteht nur aus den
  Abrufen.
* Die Filterwörter für das Activity-Log sind noch nicht mit einem echten Konto über alle Dokumentarten
  belegt; fehlende Dokumentarten melden wir als Issue nach.
