# Advanzia-Konnektor

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zum Anbieter Advanzia Bank. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen des Anbieters verstoßen.
> This connector is an independent community project and is not affiliated with Advanzia Bank. Interface: unofficial. Using unofficial interfaces may violate the provider's terms of service.
<!-- sync:connector-note:end -->

Der Konnektor unterstützt Kreditkarten mit Zugang über `https://mein.advanzia.com/`.
Die Unterstützung richtet sich nach dem verwendeten Portal, nicht nach dem Namen
auf der Karte. Andere Advanzia-Portale sind nicht abgedeckt.

## Funktionsumfang

- Ersteinrichtung mit Benutzername, Passwort und Auswahl SMS oder Advanzia-App.
- Optionale lokale Passwortspeicherung im Apple-Schlüsselbund.
- Derselbe Hintergrundablauf bei Einrichtung und späteren Abrufen.
- Native Eingabe eines SMS-Codes beziehungsweise Hinweis auf die App-Freigabe.
- Explizit zugeordnetes Kreditkartenkonto, Bankkontostand und Kartenumsätze.
- Stabile Buchungskennungen und konservative Behandlung von Vormerkungen.

- PDF-Monatsabrechnungen beim regulären Abruf und manuell unter
  „Dokumente / Kontoauszüge“ auf macOS, iOS und Linux.

Der Zugang unterstützt eine Karte pro Login. macOS und iOS nutzen WebKit,
Linux das sichtbare Qt-WebEngine-Fenster mit manueller Bank-Anmeldung.
Der Konnektor bleibt in der Bankmatrix als `TestingWanted` gekennzeichnet.

## Architektur und Sicherheitsgrenzen

`connectors/advanzia.rs` stellt einen Browseradapter für `BankConnector` bereit.
Der normale Rust-Anmeldepfad signalisiert `BrowserLoginRequired`. Die Swift-Oberfläche
führt die WebKit-Anmeldung aus und importiert anschließend über
`Vault.import_advanzia_capture` ausschließlich ausgewählte Umsatzfelder und einen
geprüften skalaren Bankkontostand. Ein fehlender Kontostand wird nicht geschätzt.
Ein gemeldeter Nullsaldo ist ein gültiger Bankkontostand.

`AdvanziaWorkflow` besitzt den Ablauf unabhängig vom sichtbaren Dialog. Die
WebKit-Ansicht bleibt im Hintergrund eingebettet. Die vollständige Bankseite
öffnet sich nur nach ausdrücklicher Auswahl als Ausweichmöglichkeit. Der Workflow
übernimmt Daten nach Anmeldung und Kontozuordnung automatisch; ein fehlgeschlagener
Import wird nicht in einer Schleife wiederholt.

Jeder Zugang erhält im verschlüsselten Tresor einen zufälligen Profil-Namensraum.
Dieser trennt WebKit-Sitzungen und Schlüsselbundeinträge. Ein geänderter Benutzername
wechselt den Namensraum. Tresorwechsel und Abbruch beenden den laufenden Workflow.
Passwörter bleiben lokal, sind nicht synchronisierbar und werden nicht in den
Rust-Konnektor, Umsatzdaten oder Protokolle übernommen. Tests verwenden einen
injizierten Schlüsselbund im Arbeitsspeicher.

Die Automatisierung prüft die exakten HTTPS-Ursprünge `id.advanzia.com` für die
Anmeldung und `mein.advanzia.com` für Umsätze. Ein SMS-Code wird ausschließlich
nach manueller Eingabe an ein eindeutig erkanntes Formular übergeben. Er wird
nicht ausgelesen, gespeichert oder protokolliert. Der zusätzliche Abruf innerhalb
der angemeldeten Sitzung verwendet GET und folgt keinen Weiterleitungen.

Neue Zugriffe sind die WebKit-Verbindung zu diesen Bankdiensten, ein lokaler
WebKit-Datenspeicher und der lokale Apple-Schlüsselbund. Neue Prozesse und neue
Paketabhängigkeiten werden dafür nicht eingeführt.

## Kontozuordnung, Duplikate und Vormerkungen

Das erste Bankkonto wird ausdrücklich einem neuen oder geeigneten vorhandenen
Kreditkartenkonto zugeordnet. Unterschiedliche Bankkennungen dürfen nicht
unbemerkt auf dasselbe Konto zeigen. Bekannte Konto- und Buchungskennungen aus
früheren experimentellen Versionen werden bei eindeutig bestätigter Zuordnung
erhalten. Gelöschte Buchungen und widersprüchliche Kennungen werden berücksichtigt.
Die Duplikaterkennung beruht nicht allein auf Betrag, Datum und Händler.

Vormerkungen beeinflussen einen gemeldeten Bankkontostand und dessen laufende
Salden nicht. Ein fehlender Eintrag in einem unvollständigen Umsatzfenster löscht
keine Vormerkung. Die automatische Ablösung setzt eine eindeutige Zuordnung in
beiden Richtungen voraus; Datum, Betrag und normalisierter Händler dienen hierbei
nur der konservativen Ablösung einer Vormerkung, nicht als Buchungsidentität.

Bei der Ablösung bleiben Kategorie, Notiz und Prüfmarkierung erhalten, soweit
die gebuchte Zeile noch keinen entsprechenden Wert besitzt. Tags werden
zusammengeführt. Eine bestehende Kategorie der gebuchten Zeile wird nicht
überschrieben. Splits oder Dokumente verhindern die automatische Ablösung.
Mehrdeutige Fälle bleiben zur Prüfung sichtbar. Alle Änderungen erfolgen in
derselben Importtransaktion.

## PDF-Monatsabrechnungen (#1975)

Der reguläre Abruf übernimmt nach Saldo und Umsätzen fehlende PDFs aus dem
aktuellen Kalendermonat und den 23 Vormonaten. Manuell stehen „Letzte 12 Monate“
und „Alle Kontoauszüge laden“ zur Auswahl. Die Historie endet beim ältesten
vom Portal gelieferten Dokument. Die Einstellung zum Dokumentenabruf des
Bankzugangs gilt auch für Advanzia.

Die Abrechnungsliste kommt aus `/api/accounts/{accountId}/statements`.
Einzelne PDFs kommen aus demselben Pfad mit angehängter `statementId`.
Beide Anfragen laufen in der vorhandenen Sitzung als Same-Origin-GET ohne
Redirects. Die Browser-Bridge begrenzt die Liste auf 512 KiB bzw. 600 Einträge,
und jedes PDF auf 8 MiB. Sie prüft HTTP-Status, MIME-Typ und PDF-Signatur.
Der Core validiert die Daten erneut vor der Ablage. Der Abruf verwendet
das bestätigte Konto und die vorhandene Dokumentenablage des Tresors.

Jahr und Monat werden als letzter Kalendertag des Abrechnungsmonats gespeichert.
Die Bank liefert keinen genauen Abschlusstag. Der Core erkennt vorhandene PDFs
am Hash aus Konto-UUID, Abrechnungsmonat und Dateireferenz. So bleiben mehrere
Dateireferenzen pro Monat erhalten und wechselnde Download-UUIDs verursachen
keinen erneuten Download. Bei einer geänderten Dateireferenz kann das Portal
dieselbe Abrechnung erneut liefern. Die Dateireferenz wird nur zur Wiedererkennung
verwendet. Einzelne PDF-Fehler lassen bereits gespeicherte Abrechnungen sowie
den Umsatz- und Saldoimport verfügbar.

Die inoffizielle API wurde in einer angemeldeten Edge-Sitzung geprüft.
Dabei waren 23 Monatsabrechnungen verfügbar und beide Stichproben hatten die
erwartete PDF-Signatur. Maximale Historie, langfristige Stabilität der
Dateireferenz und vollständige PDF-Integrität sind offen. Bank-Live-Tests
in WebKit und Qt WebEngine stehen aus.

## Tests ohne Bankverbindung

Alle automatisierten Advanzia-Tests verwenden erfundene Daten:

```sh
GIROCRAB_SKIP_SWIFT=1 cargo test -q -p girocrab-core advanzia
node --test scripts/test-advanzia-portal.cjs
cd GiroCrabApp
swift test -q --filter Advanzia
```

Vor Swift-Tests müssen die generierten UniFFI-Bindings zum Rust-Stand passen
(`./build-xcframework.sh --check --need-ios`). WebKit-Tests laden ausschließlich
lokale Testseiten. Abgedeckt sind unter anderem Ursprungskontrolle,
Anmeldezustände, manuelle SMS-Übergabe, App-Auswahl, automatische Übernahme,
Schlüsselbundfehler, Profiltrennung, explizite Kontozuordnung, Nullsaldo,
Kennungskompatibilität, Tombstones und Metadaten bei der Ablösung von Vormerkungen.

Die Offline-Tests beweisen keine dauerhafte Kompatibilität mit dem Bankportal.
Änderungen am Portal können Anpassungen an Erkennung und Skripten erfordern.
