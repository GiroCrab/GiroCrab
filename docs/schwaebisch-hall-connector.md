# Schwäbisch-Hall-Anbindung („Mein Konto“)

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zur Schwäbisch Hall. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen der Bank verstoßen.
> This connector is an independent community project and is not affiliated with Schwäbisch Hall. Interface: unofficial. Using unofficial interfaces may violate the bank's terms of service.
<!-- sync:connector-note:end -->

Status: **Rückmeldungen mit echten Verträgen erwünscht** – `ConnectorKind::SchwaebischHall`
(Schlüssel `schwaebischhall`), Anmeldung im Browserfenster, Auswertung in
`girocrab-core/src/connectors/schwaebisch_hall.rs`, Abruf-Skripte in
`GiroCrabApp/Sources/GiroCrabUI/SchwaebischHallUI.swift`.
Issue: [#290](https://github.com/GiroCrab/GiroCrab/issues/290).

Die Bausparkasse Schwäbisch Hall (BLZ 62220000) bietet keine API für Kundinnen und Kunden.
Bausparverträge, Umsätze und das Postfach sind im Kundenportal „Mein Konto“ auf
`https://www.schwaebisch-hall.de` zu sehen. Der Connector liest dieselben JSON-Antworten,
die das Portal selbst lädt. Unten stehen nur Pfade, Feldnamen und Regeln, keine echten Werte.

## Zugang

* Im Assistenten „Bank hinzufügen“ per „Schwäbisch Hall“, „Bausparvertrag“, „BSH“ oder BLZ zu finden.
* Der Name der Verbindung ist frei wählbar. Zugangsdaten speichert GiroCrab nicht: Die Anmeldung
  (Benutzername, Passwort und gegebenenfalls ein zweiter Faktor) läuft im eingebetteten Browser
  auf den Seiten der Bausparkasse.
* Die Sitzungs-Cookies bleiben wie bei den anderen Portal-Anbindungen im Browser-Datenspeicher
  der App; das Portal beendet die Sitzung nach einiger Zeit selbst.

## Ablauf

| Schritt | Anfrage | Auswertung |
|---|---|---|
| 1 | `GET /bin/bshweb/authentication` | Startet die Anmeldung: Weiterleitung zu `login.schwaebisch-hall.de` (Azure AD B2C, Policy `B2C_1A_SEAMLESSMIGRATION_SIGNUPORSIGNIN`, Code-Flow mit PKCE). Nach der Anmeldung setzt das Portal die Sitzungs-Cookies. |
| 2 | `GET /bin/bshweb/accountcontracts` | Verträge: `type`, `bsn`, `accountBalance`, `vtn`. Ohne Sitzung kommt `{"status":"error", …"code":"timeout"}`; daran erkennt das Fenster, ob man angemeldet ist. |
| 3 | `GET …/myaccount_accounts.account_details.json?vtn={vtn}` | Je Vertrag: `contracttype`, `contractphase`, `transactions` (nach Jahr und Monat gruppiert, je Buchung `day`, `subject`, `amount`). |
| 4 | `GET …/nachrichten.bin.postboxmessage?command=getmessages` | Postfach (nur wenn „PDF-Dokumente aus dem Postfach“ an ist). Die Anhang-ID steht im Download-Formular der Nachricht. |
| 5 | `GET /bin/bshweb/postbox/attachment?id={id}` | PDF-Anhang. |

Die Abrufe laufen als Skript in der angemeldeten Seite (gleiche Herkunft, `credentials:
same-origin`, keine Weiterleitungen). An die App gehen nur die oben genannten Vertragsfelder
und die PDFs, keine Cookies, Namen, Adressen oder Lastschriftkonten.

## Regeln

* **Konten:** ein Konto je Vertrag, Kennung `bsh:{vtn}`, Name aus Vertragsart und
  Vertragsnummer (`bsn`). Sparphase → Sparkonto; Verträge mit „Darlehen“ in Art oder Phase →
  Darlehen mit negativem Saldo.
* **Beträge:** formatierte Zeichenketten wie `&#43; 1.234,56`; HTML-Entities werden dekodiert,
  Rechnen nur in Cent.
* **Datum:** Jahr und Monat kommen aus der Gruppierung (Schlüssel wie `2025`/`Dezember` oder
  Felder `year`/`month`), der Tag aus `day`. Steht im Tag ein volles Datum, gilt dieses.
* **Buchungs-IDs:** Das Portal liefert keine. Buchungen bekommen Inhalts-IDs (Datum, Betrag,
  Buchungstext); zwei gleiche Buchungen am selben Tag bleiben zwei.
* **Unvollständige Antworten:** Eine Buchung ohne erkennbares Datum oder mit unlesbarem Betrag
  verwirft den ganzen Abruf, statt Teildaten zu speichern.
* **Dokumente:** Ein PDF wird dem Vertrag zugeordnet, dessen Nummer in der Nachricht steht
  (genau einer), sonst dem ersten Vertrag der Verbindung. Bereits geladene Anhänge werden
  nicht erneut geladen. Nur PDFs werden gespeichert.
* Hintergrund-Abrufe sind nicht möglich: Die Anmeldung braucht das Browserfenster.

## Offene Punkte (bitte mit echtem Vertrag prüfen)

* Die genaue Form von `transactions` (Objekte nach Jahr/Monat oder Listen) war nur beschrieben,
  nicht als vollständige Antwort zu sehen. Der Parser akzeptiert beides; ein anonymisiertes
  Beispiel hilft, ihn festzuzurren.
* Wo die Anhang-ID steht (im `text` der Nachricht oder in eigenen Feldern) und welchen
  Dateityp der Download liefert, ist nicht bestätigt.
* Ob Darlehensverträge ihren Saldo positiv oder negativ melden.

## Tests

```bash
cargo test -q -p girocrab-core --lib schwaebisch
node --test scripts/test-schwaebisch-hall-portal.cjs
```
