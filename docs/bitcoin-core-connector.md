# Bitcoin-Core-Anbindung

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zur Bitcoin Core. Schnittstelle: offiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen der Bank verstoßen.
> This connector is an independent community project and is not affiliated with Bitcoin Core. Interface: official. Using unofficial interfaces may violate the bank's terms of service.
<!-- sync:connector-note:end -->

Status: **Tests erwünscht** – `ConnectorKind::BitcoinCore` (Schlüssel `bitcoincore`),
lokale JSON-RPC-Abfrage in `girocrab-core/src/connectors/bitcoin_core.rs`, keine TAN.

Der Connector liest den Bestand einer Wallet im **eigenen** Bitcoin-Core-Knoten. Über Adressen
und Bestand verlässt nichts den Rechner: Abgefragt wird nur der eigene Knoten auf diesem Rechner.
Von außen kommt allein der Euro-Kurs (öffentlicher Ticker, ohne Konto, ohne Angaben zur Wallet).

## Voraussetzungen

* Der Knoten läuft beim Abruf, mit eingeschaltetem RPC-Server: in Bitcoin-Qt
  *Einstellungen › Optionen › Allgemein › RPC-Server aktivieren*, oder `server=1` in der
  `bitcoin.conf` im Datenverzeichnis.
* Der Knoten schreibt dann die Datei `.cookie` in sein Datenverzeichnis; damit meldet sich
  GiroCrab an. Kein Passwort nötig.

## Zugang

* **Login:** Wallet-Name. Leer = die einzige Wallet, die der Knoten geladen hat.
* **Anmeldung:** über die `.cookie`-Datei; GiroCrab fragt nach keinem Passwort.
* **Einstellungen:** `bitcoin_core.datadir` (Datenverzeichnis, leer = Standardordner, z. B. auf
  dem Mac `~/Library/Application Support/Bitcoin`) und `bitcoin_core.rpc_url`
  (leer = `http://127.0.0.1:8332`). Auf dem Mac übernimmt GiroCrab beim ersten Abruf das
  Datenverzeichnis, das Bitcoin-Qt verwendet (`strDataDir` in dessen Einstellungen), wenn noch
  keines eingetragen ist; es steht danach in den Einstellungen und lässt sich ändern.

Die Wallet wird ein Depot (`bitcoin-core:<Wallet>`, EUR) mit einer Position „Bitcoin“ (Ticker
`BTC`, direkt gehaltene Kryptowährung). Wert = Menge × aktueller Kurs.

## Mehrere Geräte

Der Tresor kommt per iCloud-Sync auch auf Geräte ohne Knoten. Dort ruft GiroCrab Bitcoin Core
nicht ab (weder einzeln noch bei „Alle abrufen“), sondern zeigt den Hinweis, dass der Bestand per
iCloud-Sync vom Mac mit Bitcoin Core aktualisiert wird. „Kein Knoten“ heißt: Weder das
eingetragene Datenverzeichnis noch das von Bitcoin-Qt noch der Standardordner existiert. Wo der
Ordner da ist, der Knoten aber nicht läuft, bleibt es ein Fehler. Neu hinzufügen lässt sich der
Zugang nur, wenn der Knoten antwortet.

## Ablauf

| Schritt | Anfrage | Auswertung |
|---|---|---|
| 1 | `listwallets` | geladene Wallets; ohne Wallet-Namen muss es genau eine sein |
| 2 | `getbalances` an `/wallet/<Name>` | `mine.trusted` + `untrusted_pending` + `immature` |
| 3 | `GET https://api.kraken.com/0/public/Ticker?pair=XBTEUR` | letzter Handelskurs `c[0]` |

## Regeln

* Die RPC-Adresse muss auf diesem Rechner liegen (`127.0.0.1`, `::1` oder `localhost`);
  jede andere wird abgelehnt, damit Cookie oder Passwort den Rechner nie verlassen.
  Weiterleitungen werden nicht verfolgt.
* Ist der Kurs nicht abrufbar, bricht der Abruf ab; der letzte Bestand bleibt stehen.
* Eine leere Wallet ist ein vollständiger, leerer Bestand.
* Buchungen gibt es nicht: Der Knoten kennt Beträge, keine Verwendungszwecke.
