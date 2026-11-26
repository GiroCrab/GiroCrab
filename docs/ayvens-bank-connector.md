# Ayvens-Bank-Anbindung

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zum Anbieter Ayvens Bank. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen des Anbieters verstoßen.
> This connector is an independent community project and is not affiliated with Ayvens Bank. Interface: unofficial. Using unofficial interfaces may violate the provider's terms of service.
<!-- sync:connector-note:end -->

Status: **Test gesucht**. Mit echtem Konto geprüft (November 2026): Anmeldung mit SMS-Code, Saldo,
Zinsen und Einzahlungen eines Tagesgeldkontos, alle Buchungen seit der Kontoeröffnung beim ersten
Abruf und keine doppelten Buchungen beim zweiten. Auszahlungen und Logins mit mehreren Konten sind
noch nicht geprüft (siehe Offene Punkte). `ConnectorKind::AyvensBank` (Schlüssel `ayvens`), direkter
HTTP-Login in `girocrab-core/src/connectors/ayvens.rs`, kein Browserfenster.

Die Ayvens Bank N.V. (bis Oktober 2024 LeasePlan Bank) bietet weder FinTS noch einen Zugang über
Enable Banking an und veröffentlicht keine API für Kundinnen und Kunden. Der Connector nutzt die
JSON-Schnittstelle, mit der das Online-Banking unter `https://sparen.ayvensbank.de` arbeitet
(Next.js mit Auth.js).

## Zugang

* **Login:** Benutzername des Online-Bankings, **Passwort:** Passwort des Online-Bankings.
* Danach schickt die Bank einen **SMS-Code** an die hinterlegte Mobilnummer; GiroCrab fragt ihn ab.
* Ein Konto je Tagesgeldkonto (`ayvens:<id>`), Kontoart „Tagesgeld / Sparen“, EUR, IBAN aus `productNumber`.

## Ablauf

| Schritt | Anfrage | Auswertung |
|---|---|---|
| 1 | `GET /api/auth/csrf` | `csrfToken` und das zugehörige Cookie. |
| 2 | `POST /api/auth/callback/credentials` (Formular: `username`, `password`, `csrfToken`, `callbackUrl`; Kopfzeile `X-Auth-Return-Redirect: 1`) | `{url}`; enthält sie `error=`, ist die Anmeldung abgelehnt. Sonst setzt die Bank das Sitzungs-Cookie. |
| 3 | `GET /api/auth/session` | `user.username` (für die Anmeldung mit SMS-Code). |
| 4 | `POST /api/savings/send-sms` | Die Bank verschickt den SMS-Code. |
| 5 | `POST /api/auth/callback/otp-auth` (Formular: `username`, `otp`, `trustBrowser=false`, `csrfToken`, `callbackUrl`) | Danach muss die Sitzung `isOtpVerified = true` melden. |
| 6 | `GET /api/savings/account-current` | `accounts[]`: `id`, `productNumber` (IBAN), `amount` (Saldo), `interestRate`, `accruedInterest`; `totalBalance`. |
| 7 | `GET /api/savings/transactions` | Liste aller Buchungen des Logins, neueste zuerst: `date` (`JJJJ-MM-TTT00:00:00.000Z`), `amount` (mit Vorzeichen, Abgänge negativ), `reference` (IBAN des Referenzkontos oder leer), `description` (z. B. „Zinsen“ oder leer). Ohne Zeitraumfilter und ohne Kontoangabe; der Connector übernimmt die Buchungen ab dem Startdatum des Abrufs. |
| 7a | `GET /api/savings/account-transactions?accountId=<id>` | Dieselbe Liste je Konto. Für ein Tagesgeldkonto blieb sie live leer; der Connector nutzt sie nur, wenn ein Login mehrere Konten hat. |
| 8 | `POST /api/savings/logout` | Am Ende, ohne Auswertung. |

## Regeln

* Buchungsdatum ist der Kalendertag aus `date`. Verwendungszweck ist `description`, sonst
  „Einzahlung“ oder „Auszahlung“ nach dem Vorzeichen. Mit `reference` steht die IBAN des
  Referenzkontos als Gegenkonto in der Buchung, sonst ist „Ayvens Bank“ die Gegenpartei.
* Das Feld `id` der Liste ist **keine** Buchungs-ID: alle Zinsbuchungen tragen dieselbe, Einzahlungen
  die IBAN des Referenzkontos. Die Buchungen bekommen deshalb Inhalts-IDs vom Tresor.
* Passwort, SMS-Code und Sitzungs-Cookie gehen nur an `https://sparen.ayvensbank.de`; Weiterleitungen
  werden nur innerhalb dieses Hosts verfolgt.
* Eine Buchung ohne lesbares Datum oder ohne lesbaren Betrag verwirft den ganzen Abruf, ebenso ein Konto
  ohne Saldo oder eine Umsatzantwort, die keine Liste ist.
* Jeder Abruf fragt den SMS-Code ab: zwischen zwei Abrufen bleibt keine Sitzung gespeichert.

## Offene Punkte

* Auszahlungen wurden noch nicht mit echten Daten gesehen.
* Logins mit mehreren Konten (z. B. Sparziele): ungeprüft, ob die Liste je Konto dann Buchungen liefert.
* Festgeld (`/api/savings/term-deposits`) wird nicht abgerufen: der Aufbau der Antwort ist unbekannt.
* Die Anmeldung per QR-Code mit der Ayvens-App wird nicht unterstützt.
