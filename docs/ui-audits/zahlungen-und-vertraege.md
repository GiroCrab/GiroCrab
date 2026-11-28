# UI-Audit: Zahlungen & Verträge

- **Ansicht:** „Geplant“, „Lastschriften“ und „Abos & Verträge“ mit ihren Listen, Aktionen und Details in der gemeinsamen Hauptnavigation.
- **Dateien:** `PlannedPaymentsView.swift`, `RootView.swift`, `DirectDebitsView.swift`, `MandateInspector.swift`, `SubscriptionsView.swift`, `SubscriptionRow.swift`, `SubscriptionInspector.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` anhand von SwiftUI-Code und Demo-Aufnahmen, dazu GiroCrab-Checkliste.
- **Nachweis:** macOS 27.2.1, iPhone 18 Pro und Pro Max sowie iPad (iOS 27.2), Deutsch. Standard, große Schrift und Schwärzung mit Demo-Daten. [Prüfbuild vom 25.11.2026](README.md#prüfbuilds).

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| GPL-1 | P1 | Adaptivität | iPhone | Bei „Accessibility XL“ ragt „Geplant“ links aus dem Bildschirm. Der Monatsbetrag ist rechts abgeschnitten und „3 Monate“ zerfällt mitten im Wort | Fix | `fixed` |
| GPL-2 | P2 | Orientierung | iPhone | Das Detail eines künftigen Dauerauftrags und das Mandatsdetail heißen im Sheet jeweils „Buchung“ | Fix | `fixed` |
| LAS-1 | P1 | Adaptivität | iPhone | Bei „Accessibility XL“ füllen Kopf und Aktionen den sichtbaren Bereich. Beim Öffnen ist keine Mandatszeile sichtbar. Der Kontoname bricht mitten im Wort um und Import/Export werden gekürzt | Fix | `fixed` |
| ABO-1 | P1 | Adaptivität | iPhone | Die Ansicht ragt links und rechts aus dem Bildschirm. Schon bei Standardgröße fehlen Teile des Titels und Zeileninhalte. Mit „Accessibility XL“ sind Ein- und Ausgabensummen abgeschnitten | Fix | `fixed` |
| ABO-2 | P1 | Bedienung | iPhone | Tippen auf einen Vertrag markiert die Zeile, öffnet aber das Vertragsdetail nicht | Fix | `fixed` |

## Impeccable-Bewertung

| Ansicht | Datum / Stand | Critique | Technisches Audit | Kurzbewertung |
|---|---|---|---|---|
| Geplant | 25.11.2026, `main` @ `09f11987` | 26/36 | 13/20 | Monatsgruppen sowie getrennte Ein- und Ausgänge machen den Zeithorizont verständlich. Große Schrift erschwert die Kontrolle von Filtern und Summen. Abgleich bereits ausgeführter Zahlungen ist in [#1881](https://github.com/GiroCrab/GiroCrab/issues/1881) erfasst. |
| Lastschriften | 25.11.2026, `main` @ `09f11987` | 26/40 | 13/20 | Aktive Mandate, Rücklastschriften und inaktive Einträge sind bei Standardgröße unterscheidbar. Die Mandate geraten bei großer Schrift hinter die Einrichtungs- und Dateiaktionen. |
| Abos & Verträge | 25.11.2026, `main` @ `09f11987` | 23/40 | 10/20 | Kündigungshinweise, Turnus und Jahressummen unterstützen den Vergleich auf dem Mac. Die abgeschnittene iPhone-Ansicht erschwert sowohl die Übersicht als auch den Vergleich einzelner Verträge. |

## Prüfnachweis

### Geplant

- **Demo-Einstieg:** `--demo-seed --view geplant`, „Alle Konten“ und „3 Monate“.

- **Nachprüfung (27.11.2026):** GPL-1 und GPL-2 auf iPhone 18 Pro (iOS 27.2) vor und nach dem Fix auf Basis von `bfc5c262` geprüft. Bei „Accessibility XL“ sind Kontofilter, Zeitraum und Beträge lesbar. Das Detail zu „Fitnessstudio Mitte“ heißt „Dauerauftrag · wöchentlich“. Der Titel bleibt bei Schwärzung lesbar, die persönlichen Felder sind verdeckt.

### Lastschriften

- **Demo-Einstieg:** `--demo-seed --view lastschriften`, Gläubigerkonto „Musterbank Girokonto“. Im iPhone-Simulator von Standardgröße auf „Accessibility XL“ wechseln.

- **Nachprüfung (27.11.2026):** LAS-1 und GPL-2 auf dem iPhone 18 Pro (iOS 27.2) auf Basis von `bfc5c262` geprüft. Bei „Accessibility XL“ ist die Kontowahl lesbar und bedienbar. Nach dem Scrollen lässt sich „Jan Muster“ öffnen. Das Sheet heißt „Mandat“.

### Abos & Verträge

- **Demo-Einstieg:** `--demo-seed --view abos`. „Original“ als Betragsbasis belassen. Vertragskopf und erste Ausgabenzeile auf iPhone bei Standardgröße und „Accessibility XL“ vergleichen.

- **Nachprüfung (27.11.2026):** ABO-1 und ABO-2 auf dem iPhone 18 Pro (iOS 27.2) vor und nach dem Fix auf Basis von `bfc5c262` geprüft. Bei Standard und „Accessibility XL“ bleiben Titel, Filter und Beträge im sichtbaren Bereich. Bei „Original“ sind die Währungsbeträge vollständig lesbar. „Autobank“ öffnet und schließt das Sheet „Vertrag“. Der separate iCloud-Sync-Fall ist in [#1918](https://github.com/GiroCrab/GiroCrab/issues/1918) erfasst.
