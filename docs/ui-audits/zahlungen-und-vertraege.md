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

#### GPL-1 · Geplante Zahlungen laufen aus dem sichtbaren Bereich

- **Ort:** `PlannedPaymentsView.swift`, Filter mit festen Breitenbereichen und Monatskopf.
- **Beobachtung:** Auf dem iPhone bei „Accessibility XL“ ragt „Geplant“ links aus dem Bild, der Monatsbetrag rechts. „3 Monate“ bricht mitten im Wort um.
- **Vorschlag:** Filter bei beengter Breite stapeln. Monatsbeträge auf eine eigene Zeile setzen, wenn Beschriftung und Summe nebeneinander keinen Platz haben.
- **Prüfung:** „Alle Konten“ und „3 Monate“ mit der genannten Schriftstufe prüfen. Filter, Titel, Monatsbetrag und erste Zeile bleiben lesbar.

#### GPL-2 · Detailtitel „Buchung“ bezeichnet zwei andere Vorgänge

- **Ort:** `RootView.swift`, `MainSplitView.bookingSheetTitle`.
- **Beobachtung:** Auf dem iPhone bei Standardgröße heißen die Details der geplanten Zahlung „Fitnessstudio Mitte“ und des Mandats „Jan Muster“ jeweils „Buchung“.
- **Vorschlag:** Titel nach dem dargestellten Vorgang wählen, etwa „Geplante Zahlung“ und „Lastschriftmandat“. Personenbezogene Titelanteile müssen bei Schwärzung verdeckt bleiben.
- **Prüfung:** Beide Details öffnen und die Titel in Standardgröße sowie mit Schwärzung prüfen.

### Lastschriften

- **Demo-Einstieg:** `--demo-seed --view lastschriften`, Gläubigerkonto „Musterbank Girokonto“. Im iPhone-Simulator von Standardgröße auf „Accessibility XL“ wechseln.

- **Detail und Schwärzung:** Bei Standardgröße „Jan Muster“ öffnen. Der Rücklastschrift-Hinweis steht auch im Detail. Der Titel „Buchung“ ist in [GPL-2](#befunde) erfasst. Die Liste verdeckt in der geschwärzten Variante Namen und Beträge.

#### LAS-1 · Mandate sind bei großer Schrift unter dem festen Kopf verborgen

- **Ort:** `DirectDebitsView.swift`, `controls`, `accountPicker` und die separate Listen-`ScrollView`.
- **Beobachtung:** Auf dem iPhone bei „Accessibility XL“ füllen Kopf und Aktionen die sichtbare Fläche. Beim Öffnen ist kein Mandat sichtbar. Kontoname und Dateiaktionen werden umgebrochen oder gekürzt.
- **Vorschlag:** Kopf und Mandate gemeinsam scrollen lassen und schmale Aktionsgruppen stapeln.
- **Prüfung:** „Musterbank Girokonto“ bei „Accessibility XL“ öffnen, bis zum ersten Mandat scrollen und es öffnen. Konto- und Aktionsbeschriftungen sind vollständig lesbar.

### Abos & Verträge

- **Demo-Einstieg:** `--demo-seed --view abos`. „Original“ als Betragsbasis belassen. Vertragskopf und erste Ausgabenzeile auf iPhone bei Standardgröße und „Accessibility XL“ vergleichen.

#### ABO-1 · Vertragsansicht ist breiter als der Bildschirm

- **Ort:** `SubscriptionsView.swift`, `totals` und `controls`.
- **Beobachtung:** Ein- und Ausgaben stehen nebeneinander und halten Betrag und Turnus per `.fixedSize()` zusammen. Mehrere Menüs bleiben auch im schmalen Zweig nebeneinander. Bei Standardgröße sind Titel und Zeilen angeschnitten, bei „Accessibility XL“ auch die Summen.
- **Vorschlag:** Summenspalten und Filter auf kleinen Breiten stapeln. Währungsbeträge zusammenhalten und die Zeilenbreite begrenzen.
- **Prüfung:** Auf dem iPhone mit „Original“ bei Standard und „Accessibility XL“ Titel, erste Zeile, Such- und Filteraktionen sowie vollständige Summen pro Währung prüfen.

#### ABO-2 · Tippen auf Vertrag öffnet kein Detail

- **Ort:** `SubscriptionRow.swift`, `SubscriptionRow`, sowie `RootView.swift`, `MainSplitView.showBookingSheet`.
- **Beobachtung:** Auf dem iPhone „Autobank“ antippen. Die Zeile wird ausgewählt. Das kompakte Detail bleibt zu, während der Mac die Darlehensrate im Inspektor zeigt.
- **Vorschlag:** Beim Antippen in der kompakten Darstellung zusätzlich das Detail-Sheet öffnen und den bestehenden Auswahlpfad beibehalten.
- **Prüfung:** Auf dem iPhone „Autobank“ öffnen, Rate und Kündigungsfrist prüfen und das Detail schließen. Auf dem Mac die Auswahl im Inspektor nachprüfen. Die iCloud-Synchronisierung bleibt in [#1918](https://github.com/GiroCrab/GiroCrab/issues/1918) erfasst.
