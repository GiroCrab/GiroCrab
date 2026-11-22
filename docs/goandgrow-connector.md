# Go-&-Grow-Anbindung

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zur Go & Grow. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen der Bank verstoßen.
> This connector is an independent community project and is not affiliated with Go & Grow. Interface: unofficial. Using unofficial interfaces may violate the bank's terms of service.
<!-- sync:connector-note:end -->

Status: **verifiziert mit echtem Konto** (November 2026) – `ConnectorKind::GoAndGrow` (Schlüssel `goandgrow`), direkter
HTTP-Login in `girocrab-core/src/connectors/goandgrow.rs`, kein Browserfenster, keine TAN.

Go & Grow (ehemals Bondora Go & Grow) veröffentlicht keine API für Anlegerinnen und Anleger.
Der Connector meldet sich wie die Web-App unter `https://app.goandgrow.eu` an und liest
dieselbe JSON-Schnittstelle (`https://api.prd.goandgrow.eu/investor/api/v2`).

## Zugang

* **Login:** E-Mail-Adresse des Go-&-Grow-Kontos, **Passwort:** Go-&-Grow-Passwort.
* Ein Konto `goandgrow:<Id>`, Kontoart „Tagesgeld / Sparen“, EUR. Kontonummer ist die
  führende Zahl der `ReferenceNumber`.
* **Die Schnittstelle liefert nur die letzten zehn Buchungen**, Zinsen wie Ein- und
  Auszahlungen. Abrufen, bevor mehr als zehn neue Buchungen anfallen, sonst entsteht eine
  Lücke; bei täglichen Zinsen ohne weitere Buchungen heißt das spätestens alle zehn Tage.
  Ältere Buchungen lassen sich nur per Import übernehmen.

## Ablauf

| Schritt | Anfrage | Auswertung |
|---|---|---|
| 1 | `GET https://app.goandgrow.eu/en/login` | Weiterleitung zum Keycloak-Anmeldeformular `form#kc-form-login` auf `sso.goandgrow.eu`. |
| 2 | `POST <action des Formulars>` | Felder `username`, `password`, `credentialId` (leer) und die versteckten Felder. **Genau einmal** senden: der `session_code` in der Adresse gilt nur einmal. Antwort ist eine Seite mit einem Formular an `/signin-oidc` (OIDC `form_post`). Erscheint wieder das Anmeldeformular, sind die Zugangsdaten falsch. |
| 3 | `POST https://app.goandgrow.eu/signin-oidc` | Felder des Formulars aus Schritt 2 (`code`, `id_token`, `state`, `session_state`). Die App setzt ihre Sitzungs-Cookies. |
| 4 | `GET …/investor/api/v2/gogrow` | Liste mit dem Konto: `Id`, `ReferenceNumber`, `Name`, `ClientValue` (Saldo). |
| 5 | `GET …/investor/api/v2/statements` | Die letzten zehn Buchungen: `Type`, `Description`, `Amount`, `Date`, `Balance`. |

Die Schnittstelle authentifiziert allein über die Cookies, ohne `Authorization`-Kopfzeile.

## Regeln

* `Amount` ist immer positiv. `Withdrawal`, `WithdrawalRequest`, `PartialPayout` und `Fee`
  werden als Belastung gebucht, `Return` (Zinsen) und `Deposit` (Einzahlung) als Gutschrift.
* Buchungsdatum ist der Kalendertag aus `Date` (`JJJJ-MM-TTThh:mm:ss`, teils mit Millisekunden).
  Gegenpartei ist „Go & Grow“, Verwendungszweck ist `Description` (sonst der Typ).
* Die Schnittstelle liefert keine Buchungs-IDs: Buchungen bekommen Inhalts-IDs
  (`RemoteTransaction::content_id`).
* Weiterleitungen werden von Hand verfolgt und nur zwischen `app.goandgrow.eu`,
  `sso.goandgrow.eu`, `sso.bondora.com` und `api.prd.goandgrow.eu` (jeweils `https`).
  Passwort und Cookies erreichen keinen anderen Host; Formulare mit `GET` werden abgelehnt.
* Nennt eine Weiterleitung einen dieser Hosts mit `http` (die App steht hinter einem Proxy),
  wird sie als `https` aufgerufen; unverschlüsselt geht nichts hinaus.
* Eine Buchung ohne lesbares Datum, ohne lesbaren Betrag oder mit unbekanntem Typ verwirft den ganzen Abruf, ebenso eine Antwort, die keine Liste ist.

## Offene Punkte

* Konten mit Zwei-Faktor-Anmeldung sind nicht berücksichtigt.
* Die Web-App bietet einen Export der ganzen Historie (`statements/file`, XLSX/PDF); der
  Connector nutzt ihn nicht.
