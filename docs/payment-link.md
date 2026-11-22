# Überweisung per Link (`girocrab://transfer`)

Andere Apps und Webseiten können GiroCrab bitten, ein Überweisungsformular mit Empfänger, IBAN, Betrag und Verwendungszweck zu öffnen, etwa eine Belegverwaltung oder eine Rechnung im Browser. Die Belegverwaltung [Receipts Space](https://receipts-app.com/) nutzt den Link für „Direkt bezahlen“ (macOS): Dort GiroCrab als App zum Bezahlen wählen. GiroCrab füllt das Formular nur aus. Gesendet wird erst, wenn die Nutzerin oder der Nutzer die Überweisung im Formular prüft und mit PIN und TAN freigibt.

Gilt für macOS und iOS/iPadOS. Code: `GiroCrabApp/Sources/GiroCrabUI/PaymentLink.swift` (Parser), `AppModel+Transfer.swift` (`openLink`, `presentPendingPaymentLink`).

## Format

```
girocrab://transfer?name=Stadtwerke%20M%C3%BCnchen&iban=DE89370400440532013000&amount=85.00&purpose=Abschlag%20Strom%20Kd-Nr%20482910
```

| Parameter | Pflicht | Inhalt |
| --- | --- | --- |
| `iban` | ja | IBAN des Empfängers, Leerzeichen erlaubt |
| `name` | nein | Name des Empfängers, höchstens 70 Zeichen |
| `bic` | nein | BIC des Empfängers |
| `amount` | nein | Betrag in Euro, `85.00` oder `85,00`; nur positive Beträge |
| `purpose` | nein | Verwendungszweck, höchstens 140 Zeichen |
| `currency` | nein | nur `EUR`; jede andere Währung lehnt GiroCrab ab |

Alle Werte sind URL-kodiert (UTF-8). Zeilenumbrüche werden zu Leerzeichen, unbekannte Parameter werden ignoriert. Ein Link ohne IBAN oder mit fremder Währung öffnet kein Formular, sondern eine Fehlermeldung.

## Verhalten

- Das Formular öffnet im bestehenden Hauptfenster, mit dem Hinweis „Aus einem Link übernommen“: Empfänger, IBAN und Betrag kommen von außen und sind vor der Freigabe zu prüfen. Die Prüfung der IBAN und der Empfängerabgleich der Bank laufen wie bei jeder anderen Überweisung.
- Der Link wartet, solange das Formular nicht öffnen kann: bei gesperrtem Tresor bis zum Entsperren und bis die Konten geladen sind, während eines Abrufs oder Bankauftrags bis zu dessen Ende, bei einem schon offenen Formular (auch auf seinem Ergebnisschritt) bis es geschlossen ist, auch wenn es dabei zur Sammelüberweisung oder zurück wechselt. Ein offenes Formular wird nie überschrieben.
- Kommt ein neuer Link, während einer wartet, gilt der neue: Er ist der zuletzt geöffnete, und vom früheren wurde noch nichts angezeigt.
- Der Link nennt einen Empfänger, und vom Tagesgeldkonto geht nur eine Umbuchung. Ist ein Tagesgeldkonto ausgewählt, nimmt das Formular deshalb das erste Girokonto. Gibt es kein Girokonto mit IBAN, kommt eine Fehlermeldung statt eines Formulars ohne Empfänger.

## Testen

Der Link braucht ein App-Bundle, das das Schema `girocrab` registriert (`CFBundleURLTypes` in `GiroCrabApp/Xcode/LaunchInfo.plist` und in `package-macos-app.sh`). `swift run GiroCrabMac` startet kein Bundle; dafür die App mit `./package-macos-app.sh` oder in Xcode bauen, starten und dann im Terminal:

```sh
open 'girocrab://transfer?name=Stadtwerke%20M%C3%BCnchen&iban=DE89370400440532013000&amount=85.00&purpose=Abschlag%20Strom'
```
