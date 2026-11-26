# UI-Audit: Zahlungen & Verträge

- **Ansicht:** „Geplant“, „Lastschriften“ und „Abos & Verträge“ mit ihren Listen, Aktionen und Details in der gemeinsamen Hauptnavigation.
- **Dateien:** `PlannedPaymentsView.swift`, `RootView.swift`, `DirectDebitsView.swift`, `MandateInspector.swift`, `SubscriptionsView.swift`, `SubscriptionRow.swift`, `SubscriptionInspector.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` anhand von SwiftUI-Code und Demo-Aufnahmen, dazu GiroCrab-Checkliste.
- **Nachweis:** macOS 27.2.1, iPhone 18 Pro und Pro Max sowie iPad (iOS 27.2), Deutsch. Standard, große Schrift und Schwärzung mit Demo-Daten. [Prüfbuild vom 25.11.2026](README.md#prüfbuilds).

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| GPL-1 | P1 | Adaptivität | iPhone | Bei „Accessibility XL“ ragt „Geplant“ links aus dem Bildschirm. Der Monatsbetrag ist rechts abgeschnitten und „3 Monate“ zerfällt mitten im Wort | Issue | `open` |
| GPL-2 | P2 | Orientierung | iPhone | Das Detail eines künftigen Dauerauftrags und das Mandatsdetail heißen im Sheet jeweils „Buchung“ | Issue | `open` |
| LAS-1 | P1 | Adaptivität | iPhone | Bei „Accessibility XL“ füllen Kopf und Aktionen den sichtbaren Bereich. Beim Öffnen ist keine Mandatszeile sichtbar. Der Kontoname bricht mitten im Wort um und Import/Export werden gekürzt | Issue | `open` |
| ABO-1 | P1 | Adaptivität | iPhone | Die Ansicht ragt links und rechts aus dem Bildschirm. Schon bei Standardgröße fehlen Teile des Titels und Zeileninhalte. Mit „Accessibility XL“ sind Ein- und Ausgabensummen abgeschnitten | Issue | `open` |
| ABO-2 | P1 | Bedienung | iPhone | Tippen auf einen Vertrag markiert die Zeile, öffnet aber das Vertragsdetail nicht | Issue | `open` |

## Impeccable-Bewertung

| Ansicht | Datum / Stand | Critique | Technisches Audit | Kurzbewertung |
|---|---|---|---|---|
| Geplant | 25.11.2026, `main` @ `09f11987` | 26/36 | 13/20 | Monatsgruppen sowie getrennte Ein- und Ausgänge machen den Zeithorizont verständlich. Große Schrift erschwert die Kontrolle von Filtern und Summen. Abgleich bereits ausgeführter Zahlungen ist in [#1881](https://github.com/GiroCrab/GiroCrab/issues/1881) erfasst. |
| Lastschriften | 25.11.2026, `main` @ `09f11987` | 26/40 | 13/20 | Aktive Mandate, Rücklastschriften und inaktive Einträge sind bei Standardgröße unterscheidbar. Die Mandate geraten bei großer Schrift hinter die Einrichtungs- und Dateiaktionen. |
| Abos & Verträge | 25.11.2026, `main` @ `09f11987` | 23/40 | 10/20 | Kündigungshinweise, Turnus und Jahressummen unterstützen den Vergleich auf dem Mac. Die abgeschnittene iPhone-Ansicht erschwert sowohl die Übersicht als auch den Vergleich einzelner Verträge. |

## Prüfnachweis

### Geplant

- **Demo-Einstieg:** `--demo-seed --view geplant`, „Alle Konten“ und „3 Monate“.
- **GPL-1:** iPhone auf „Accessibility XL“ stellen. Die Filter in `PlannedPaymentsView` bleiben nebeneinander, mit festen Breitenbereichen. In `iphone-gross-geplant.png` bleiben Titel und Monatssummen am Rand unvollständig. Abnahme mit lesbaren Filtern, Monatssummen und vollständigen Zeilen. [Bildbelege (ZIP)][bildbelege]
- **GPL-2:** Auf dem iPhone bei Standardgröße „Fitnessstudio Mitte“ antippen. `MainSplitView` verwendet den Titel aus `bookingSheetTitle` auch für die geplante Zahlung. Der Titel soll den dargestellten Vorgang benennen und persönliche Namen bei Schwärzung abdecken (`iphone-geplant-detail.png`, `iphone-lastschriften-detail.png`). [Bildbelege (ZIP)][bildbelege]

### Lastschriften

- **Demo-Einstieg:** `--demo-seed --view lastschriften`, Gläubigerkonto „Musterbank Girokonto“. Im iPhone-Simulator von Standardgröße auf „Accessibility XL“ wechseln.
- **LAS-1:** `DirectDebitsView` platziert `controls` oberhalb der separaten Listen-`ScrollView`. `accountPicker` begrenzt die Breite. `iphone-gross-lastschriften.png` zeigt den mehrzeiligen Kontonamen und verdeckte Mandate. Abnahme mit erreichbaren Mandaten und vollständig lesbaren Bedienoptionen. [Bildbelege (ZIP)][bildbelege]
- **Detail und Schwärzung:** Bei Standardgröße „Jan Muster“ öffnen. Der Rücklastschrift-Hinweis steht auch im Detail. Der Titel „Buchung“ ist in [GPL-2](#befunde) erfasst. Die Liste verdeckt in der geschwärzten Variante Namen und Beträge.

### Abos & Verträge

- **Demo-Einstieg:** `--demo-seed --view abos`. „Original“ als Betragsbasis belassen. Vertragskopf und erste Ausgabenzeile auf iPhone bei Standardgröße und „Accessibility XL“ vergleichen.
- **ABO-1:** `SubscriptionsView.totals` setzt Ein- und Ausgaben nebeneinander. Beide Spalten halten Beträge und Turnus mit `.fixedSize()` zusammen. Gleichzeitig bleiben auch im schmalen Zweig von `controls` mehrere Menüs in einer Zeile. Abnahme mit erreichbaren Such- und Filteraktionen, vollständigem Titel und vollständig lesbaren Summen pro Währung (`iphone-standard-abos.png`, `iphone-gross-abos.png`). [Bildbelege (ZIP)][bildbelege]
- **ABO-2:** Nach Start auf dem iPhone „Autobank“ antippen (`iphone-abos-auswahl.png`). `SubscriptionRow` setzt die Auswahl, während das kompakte Detail in `MainSplitView` zusätzlich `showBookingSheet` benötigt. Auf dem Mac steht die Darlehensrate mit Kündigungsfrist im Inspektor. Abnahme mit öffnendem und schließbarem Vertragsdetail am iPhone. Die iCloud-Synchronisierung hat ihren eigenen offenen Punkt in [#1918](https://github.com/GiroCrab/GiroCrab/issues/1918). [Bildbelege (ZIP)][bildbelege]

[bildbelege]: https://github.com/user-attachments/files/35449133/PR-Bildbelege.zip
