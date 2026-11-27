# Amazon-Visa-Anbindung (OpenBank-Portal)

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zum Anbieter OpenBank bzw. Amazon. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen des Anbieters verstoßen.
> This connector is an independent community project and is not affiliated with OpenBank or Amazon. Interface: unofficial. Using unofficial interfaces may violate the provider's terms of service.
<!-- sync:connector-note:end -->

Status: **Rückmeldungen mit echten Verträgen erwünscht** – `ConnectorKind::AmazonVisa`
(Schlüssel `amazonvisa`), Anmeldung im Browserfenster, Auswertung der
Abrechnungsliste in `girocrab-core/src/importers/portal_statements.rs`,
Portal-Logik in `GiroCrabApp/Sources/GiroCrabUI/PortalUI.swift`.
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
  eingebetteten Browser. GiroCrab speichert Zugangsdaten nicht im Klartext;
  der gespeicherte Zugangscodes-Zugriff wird über den Tresor verwaltet.
* Nach dem Login zeigt die SPA die Abrechnungsliste unter der Route
  `/amazon/card-settings/card-statements`.

## Authentifizierung der Portal-API

* Die API auf `https://api.kunden.openbankpay.com` authentifiziert **nicht** über
  Cookies auf dem API-Host, sondern über den Custom-Header `accesstoken`.
* Die SPA liest den Wert zur Laufzeit aus dem Cookie `accessToken` der
  Portal-Domain (`document.cookie`). Der Connector liest dieses Cookie im
  Webview-Kontext und setzt den Header pro Request. Das Cookie und damit der
  Header-Wert rotieren mit der Session und werden nie geloggt oder gespeichert.

## Abrechnungsliste

* Die SPA lädt die Kontoauszugs-Liste von der Portal-API. Jeder Eintrag enthält
  das Abrechnungs-Datum und die `documentId` des zugehörigen PDFs.
* Einträge ohne eigenes Abrechnungs-Datum werden ihrer Einschreibung bzw. ihrem
  umgebenden Eintrag zugeordnet; Anhang-Dateien (z. B. im `files`-Array) gelten
  nicht als separate Abrechnung.

## PDF-Abruf (zweistufig)

1. `GET https://api.kunden.openbankpay.com/mb-api-documents/v2/documents/{documentId}/versions/1`
   → die Antwort enthält das Feld `documentUri`.
2. `documentUri` ist eine **selbst-authentifizierende, kurzlebige S3-Signed-URL**
   (Parameter `X-Amz-Security-Token` u. a., Host `*.s3.eu-west-1.amazonaws.com`).
   Das PDF wird nativ (URLSession) von dieser URL geladen und auf den
   `%PDF`-Header geprüft. Die URL wird nie geloggt.

## Konsequenzen für die Speicherung

* Die Signed-URLs sind nicht persistent speicherbar: Sie rotieren und laufen ab.
  Der Abruf erfolgt daher immer frisch über `documentId` → `documentUri` → Download.
* GiroCrab speichert weder Zugangsdaten noch die Link-URLs; nur die abgerufenen
  PDF-Abrechnungen landen wie andere Belege in der Belege-Verwaltung.
