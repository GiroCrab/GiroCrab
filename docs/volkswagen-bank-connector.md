# Volkswagen-Bank-Anbindung

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zur Volkswagen Bank. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen der Bank verstoßen.
> This connector is an independent community project and is not affiliated with Volkswagen Bank. Interface: unofficial. Using unofficial interfaces may violate the bank's terms of service.
<!-- sync:connector-note:end -->

Status: **in Entwicklung (Gerüst)** – `ConnectorKind::VolkswagenBank` (Schlüssel `volkswagenbank`),
Modul `girocrab-core/src/connectors/volkswagen_bank.rs`. Der Anmeldedialog (photoTAN, pushTAN) und die
Verdrahtung sind fertig und getestet. Die Aufrufe an das Portal fehlen noch und warten auf einen
Netzwerkmitschnitt; bis dahin meldet der Connector „noch nicht fertiggestellt“ und ruft nichts ab.

Portal: `https://banking.volkswagenbank.de/#/login` (Single-Page-App). Zweiter Faktor: photoTAN oder
pushTAN. Kein PSD2-Zugang und kein FinTS-Zugang belegt.

## Dialog (fertig)

| Schritt | Connector | Anzeige |
|---|---|---|
| `begin` | Benutzername + Passwort an das Portal | – |
| photoTAN | `Challenge` mit `ChallengeKind::ImageCode` und PNG/JPEG | Grafik scannen, Code eingeben, `respond(Code)` |
| pushTAN | `Challenge` mit `ChallengeKind::DecoupledPush`, 3 s Intervall | Freigabe in der App, `respond(Poll)` bis `Ready` |
| Gerät bekannt | direkt `Phase::Ready` | – |

Die Portal-Aufrufe stecken hinter dem Trait `Portal` (`HttpPortal` ist die einzige Stelle mit
Netzwerkzugriff). Tests laufen gegen ein skriptbares Portal.

## Offen: Platzhalter in `HttpPortal`

- [ ] `start_login` – Anmeldeseite, Cookies/Token, Anmeldung, Ergebnis auf `LoginStep` abbilden
- [ ] `poll_push` – Status der pushTAN (wartend / freigegeben / abgelehnt / abgelaufen)
- [ ] `submit_photo_tan` – Code senden, falsches Code → `CoreError::Authentication`
- [ ] `load` – Konten (Art, IBAN, Saldo) und Umsätze mit Seitenwechsel, Vorzeichen, Datumsfelder
- [ ] Wird das Gerät wiedererkannt (kein zweiter Faktor beim nächsten Login)? Dann Cookies in
      `FetchResult::state` speichern (siehe `easybank.rs`)
- [ ] Captcha, Zugriffssperre, Sitzungsablauf
- [ ] `external_id`: stabile ID oder leer lassen (Inhalts-ID aus `store_fetch`)
- [ ] Danach: `Status::Limited` in `BANK_MATRIX` auf `TestingWanted` setzen, Hinweistext im Katalog
      anpassen, README-Matrix per `scripts/sync-docs-and-website.sh` aktualisieren

## Netzwerkmitschnitt (HAR) – was gebraucht wird

Bitte im Browser (Entwicklertools → Netzwerk, „Log beibehalten“) aufzeichnen und **vor dem Teilen
schwärzen**: Passwort, Benutzername, Cookies/Tokens (`Set-Cookie`, `Cookie`, `Authorization`, XSRF),
IBANs, Namen, Beträge und Verwendungszwecke durch Platzhalter ersetzen. Feld- und Endpunktnamen,
HTTP-Methoden, Statuscodes und Header-*Namen* müssen erhalten bleiben.

1. Anmeldung mit pushTAN: Seite laden → Benutzername/Passwort → Freigabe in der App → Startseite.
2. Anmeldung mit photoTAN: wie oben, inklusive der Anfrage, die das TAN-Bild liefert.
3. Kontenübersicht und Umsatzliste (mit „mehr laden“/Seitenwechsel und Zeitraumwechsel).
4. Falsche TAN und abgelaufene pushTAN (je ein Mitschnitt).
5. Zweite Anmeldung kurz danach im selben Browser: wird die TAN erneut verlangt?
6. Abmelden.
