# Monefit-SmartSaver-Anbindung

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zur Monefit. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen der Bank verstoßen.
> This connector is an independent community project and is not affiliated with Monefit. Interface: unofficial. Using unofficial interfaces may violate the bank's terms of service.
<!-- sync:connector-note:end -->

Status: **Test gesucht** – mit echtem Konto geprüft (November 2026): Anmeldung, Saldo, Zinsen und
Einzahlungen; Auszahlungen und Gebühren noch nicht (siehe Offene Punkte) – `ConnectorKind::Monefit` (Schlüssel `monefit`), direkter
HTTP-Login in `girocrab-core/src/connectors/monefit.rs`, kein Browserfenster, keine TAN.

Monefit veröffentlicht keine API für Kundinnen und Kunden. Der Connector nutzt die
JSON-Schnittstelle, mit der die SmartSaver-Web-App unter `https://api-smartsaver.monefit.com`
arbeitet.

## Zugang

* **Login:** E-Mail-Adresse des Monefit-Kontos, **Passwort:** Monefit-Passwort.
* Ein Konto `monefit:smartsaver`, Kontoart „Tagesgeld / Sparen“, EUR.
* Eine Anmeldung mit zweitem Faktor wird nicht unterstützt; der Abruf bricht dann mit einem Hinweis ab.

## Ablauf

| Schritt | Anfrage | Auswertung |
|---|---|---|
| 1 | `POST /v1/auth/login` mit `{"identificator": <E-Mail>, "password": …}` | `data.result.token` = Bearer-Token. `data.result.mfa = true` bedeutet zweiter Faktor. |
| 2 | `GET /v1/account/summary` | `data.result.closingBalance` = Saldo (Dezimalzahl als Text, acht Nachkommastellen). |
| 3 | `GET /v1/account/list?page=N&perPage=500` | `data.result[]`, neueste zuerst: `sum`, `time` (`JJJJ-MM-TT hh:mm:ss`), `txnType`, `fee`, `description`, `balance`. `dateFrom`/`dateTo` wertet die Schnittstelle nicht aus; der Connector blättert, bis er beim Startdatum des Abrufs ankommt. |

## Regeln

* `sum` ist immer positiv. `WITHDRAWAL`, `WITHDRAW`, `PAYOUT` (Auszahlung) und `FEE` (Gebühr) werden
  als Belastung gebucht, alle anderen Typen als Gutschrift. Beobachtet wurden bisher nur `INTEREST`
  (Zinsen), `DEPOSIT` (Einzahlung) und `BONUS`; die Abgangstypen sind aus der Benennung der
  Schnittstelle abgeleitet. Eine Gebühr (`fee`) wird vom Betrag abgezogen.
* Buchungsdatum ist der Kalendertag aus `time`. Gegenpartei ist „Monefit SmartSaver“,
  Verwendungszweck ist `description` (sonst der Buchungstyp).
* Das Feld `id` der Liste ist **keine** Buchungs-ID: alle Zinsbuchungen tragen dieselbe.
  Die Buchungs-ID wird deshalb aus Zeitstempel (sekundengenau), Typ, normalisiertem Betrag und
  gegebenenfalls der Gebühr gebildet. Gleiche IDs werden nach dem Verwendungszweck sortiert
  durchnummeriert (`#1`, `#2` …), unabhängig von der Reihenfolge der Antwort. Der Verwendungszweck
  selbst gehört nicht zur ID, damit eine umformulierte Beschreibung keine neuen Buchungen erzeugt.
* Passwort und Token gehen nur an `https://api-smartsaver.monefit.com`; Weiterleitungen werden nicht verfolgt.
* Eine Buchung ohne lesbares Datum, ohne lesbaren Betrag oder ohne Buchungstyp verwirft den ganzen Abruf,
  ebenso ein fehlender Saldo oder eine Umsatzantwort, die keine Liste ist. Eine fehlende Gebühr zählt als 0.

## Offene Punkte

* Abgänge (Auszahlung, Gebühr) wurden noch nicht mit echten Daten gesehen.
* Mehrere Unterkonten („Vaults“) liefert die Schnittstelle zusammengefasst; der Connector führt ein Konto.
