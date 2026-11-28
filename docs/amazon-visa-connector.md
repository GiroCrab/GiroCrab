# Amazon-Visa-Anbindung (OpenBank-Portal)

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zum Anbieter Amazon Visa. Schnittstelle: offiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen des Anbieters verstoßen.
> This connector is an independent community project and is not affiliated with Amazon Visa. Interface: official. Using unofficial interfaces may violate the provider's terms of service.
<!-- sync:connector-note:end -->

Status: **Verified** – `ConnectorKind::AmazonVisa` (Schlüssel `amazonvisa`).
In der [Bankenmatrix](../README.md) als **EnableBanking, offiziell, Verified**
geführt — das ist die Klassifizierung, nicht der Standardweg: Der Abruf läuft
standardmäßig über das Kundenportal, den Abruf der Kontoauszugs-PDFs hat
[#1925](https://github.com/GiroCrab/GiroCrab/pull/1925) ergänzt. Der
EnableBanking-Weg (laufende Umsätze) ist optional und setzt einen ausdrücklichen
Opt-in voraus: eine eigene EnableBanking-Anwendung mit Signatur-Schlüssel, mit
der jede API-Anfrage signiert wird. Anmeldung im
Browserfenster, Auswertung der Abrechnungsliste in
`girocrab-core/src/importers/portal_statements.rs`, Portal-Logik in
`GiroCrabApp/Sources/GiroCrabUI/PortalUI.swift`.
Issues: [#1500](https://github.com/GiroCrab/GiroCrab/issues/1500) (Implementierung),
[#1924](https://github.com/GiroCrab/GiroCrab/pull/1924) (Zugangscode-Speicherung),
[#1925](https://github.com/GiroCrab/GiroCrab/pull/1925) (PDF-Abruf).

Die Amazon-Visa-Karte wird von OpenBank herausgegeben. Das Kundenportal liegt unter
`https://kunden.openbankpay.com` (Login über
`https://kunden.openbankpay.com/amazon/login`). Der Connector liest dieselben
JSON-Antworten, die das Portal selbst lädt. Unten stehen nur Pfade, Feldnamen und
Regeln, keine echten Werte.

## Zugang

* Die Anmeldung (Benutzername, Passwort, gegebenenfalls zweiter Faktor) läuft im
  eingebetteten Browser. Auf Wunsch (Option „Passwort/PIN im Tresor speichern“)
  legt GiroCrab Portal-Benutzername und Passwort (PIN) verschlüsselt im Tresor
  ab; im Klartext werden Zugangsdaten nie gespeichert.
* Nach dem Login zeigt die SPA die Abrechnungsliste unter der Route
  `/amazon/card-settings/card-statements`.

## Authentifizierung der Portal-API

* Alle Portal- und Dokument-Anfragen laufen im angemeldeten WebView: Die
  JavaScript-Fetches nutzen `credentials: 'include'` und damit die
  Session-Cookies des Portals. Eine zusätzliche Header-Behandlung ist nicht
  implementiert.
* Die Anfragen sind an den Host gebunden, von dem der Kontoauszug-Schlüssel
  stammt (`location.host`-Prüfung im Skript); der Abruf bricht ab, wenn die
  Seite inzwischen woanders steht. Cookie- und Header-Werte werden nie geloggt
  oder gespeichert.

## Abrechnungsliste

* Die SPA lädt die Kontoauszugs-Liste von der Portal-API. Jeder Eintrag enthält
  das Abrechnungs-Datum und die `documentId` des zugehörigen PDFs.
* Ein Listeneintrag ohne nutzbares Schlussdatum zählt als Fehlschlag
  (`rejected`) — ob er seinen PDF-Link oder eine `documentId` ohne Datum trägt;
  undatierte sichtbare Links ebenso.
* Datei-Objekte (z. B. im `files`-Array) sind keine Anhänge mehr: Ohne eigenes
  erkanntes Schlussdatum gehören sie zum umgebenden Eintrag (bloße Metadaten
  wie `created` zählen nicht als Schlussdatum), mit eigenem erkannten
  Schlussdatum werden sie ein eigener Eintrag mit diesem Datum.

## PDF-Abruf (zweistufig)

Beide Schritte laufen als JavaScript im WebView der angemeldeten Portalseite
(`credentials: 'include'`); das Ergebnis geht als Base64 an die App, die auf den
`%PDF`-Kopf prüft:

1. `GET {api-host}/mb-api-documents/v2/documents/{documentId}/versions/{version}`
   → die Antwort enthält das Feld `documentUri`.
2. Die `documentUri` ist eine **selbst-authentifizierende, kurzlebige
   S3-Signed-URL** (Parameter `X-Amz-Security-Token` u. a., Host
   `*.s3.eu-west-1.amazonaws.com`) und wird im zweiten Fetch (anderer Host
   erlaubt) geladen. Die URL wird nie geloggt.

Daneben können sichtbare PDF-Links der Seite (gleicher Host) direkt geladen
werden; Einträge ohne nutzbares Schlussdatum gelten als Fehlschlag.

## Konsequenzen für die Speicherung

* Die Signed-URLs sind nicht persistent speicherbar: Sie rotieren und laufen ab.
  Der Abruf erfolgt daher immer frisch über `documentId` → `documentUri` → Download.
* Portal-Benutzername und Portal-Passwort (PIN) können im Tresor gespeichert
  werden — nur, wenn die Speicherung beim Anlegen der Verbindung eingeschaltet
  ist (Option „Passwort/PIN im Tresor speichern“). Im Klartext werden
  Zugangsdaten nie abgelegt, Signed-URLs nie gespeichert.
* Die Session-Cookies des Portals bleiben im Standard-Datenspeicher des WebView
  (persistenter `WKWebsiteDataStore`) über App-Neustarts erhalten — daher
  klappt der erneute Abruf ohne neue Anmeldung. Das Abmelden im Connector
  (`clearPortalSession`) entfernt sie. Nur die abgerufenen PDF-Abrechnungen
  landen wie andere Belege in der Belege-Verwaltung.
