# Product

<!-- impeccable:product-schema 1 -->

Kontext für Design-Reviews durch Menschen und Agenten (siehe [`docs/ui-audit.md`](docs/ui-audit.md)).
Beschreibt den heutigen Stand; offene Punkte sind als solche markiert. Visuelle Regeln stehen in
[`DESIGN.md`](DESIGN.md). Verbindliche technische Regeln (`.appFont`, `.sensitive()`, `ReportDialogs`)
stehen nur in [`CLAUDE.md`](CLAUDE.md); hier und in `DESIGN.md` wird darauf verwiesen, nicht wiederholt.

## Platform

adaptive

Ein gemeinsamer Rust-Kern (`girocrab-core`), darüber native Oberflächen je Plattform:

| Plattform | Oberfläche | Stand |
|---|---|---|
| macOS | SwiftUI (`GiroCrabApp/`) | Hauptplattform |
| iOS / iPadOS | SwiftUI (`GiroCrabApp/`, gemeinsamer Code mit macOS) | TestFlight |
| Linux | Qt 6 / QML (`GiroCrabLinux/`) | Release-Pakete (`.deb`, Flatpak) |
| Windows | Slint (`GiroCrabWindows/`) | Alpha |
| Android | Kotlin (`GiroCrabAndroid/`) | Alpha |

Jede Oberfläche folgt den Konventionen ihrer Plattform. Gemeinsam sind Fachlogik, Datenmodell und
die Bedeutung der Farben und Formate (siehe `DESIGN.md`), nicht das Aussehen einzelner Bedienelemente.

**Charakter: Operate.** Alle App-Oberflächen sind ein ruhiges, dichtes Arbeitswerkzeug für wiederkehrende
Aufgaben, kein Schaufenster: viel Information auf wenig Fläche, native Plattformmuster (Seitenleiste,
Listen, Inspektor, Sheets, Menüs, Tastaturkürzel) statt eigener Bedienkonzepte, keine Inszenierung.
README und Website sind nicht Gegenstand dieser Audits.

## Users

Privatpersonen in Deutschland und Europa, die mehrere Konten, Karten und Depots bei verschiedenen
Banken haben und ihre Finanzen an einem Ort sehen und ordnen möchten. Typische Aufgaben: Umsätze
abrufen und durchsehen, kategorisieren, Belege zuordnen, Überweisungen ausführen, Salden und
Vermögen im Blick behalten.

Viele nutzen die App am Mac in längeren Sitzungen (Monatsabschluss, Belege nachtragen) und am
iPhone kurz unterwegs (Saldo prüfen, eine Buchung nachsehen, eine Überweisung freigeben).

## Product Purpose

GiroCrab ist eine freie Banking-App (GPL-3.0). Sie ruft Konten über FinTS 3.0, PSD2 und eigene
Anbindungen ab, führt SEPA-Aufträge aus und importiert vorhandene Daten. Alle Daten liegen in einem
lokal verschlüsselten Tresor auf dem Gerät; ein Abgleich zwischen Geräten läuft optional über iCloud.

Erfolg heißt: Nutzer sehen alle Konten an einem Ort, verstehen jede Buchung und jeden Fehler,
und ihre Daten verlassen das Gerät nur, wenn sie es selbst wollen.

## Positioning

- Daten bleiben auf dem Gerät, verschlüsselt; kein eigener Server, kein Konto bei GiroCrab nötig.
- Native Apps je Plattform auf einem gemeinsamen, quelloffenen Kern.
- Kostenlos und quelloffen; Funktionen werden sachlich und belegbar beschrieben, ohne Vergleiche mit anderen Produkten.

## Operating Context

- **macOS:** Fenster mit Seitenleiste (Konten, Gruppen, Kategorien), Buchungsliste und
  Inspektor rechts (Beleg, Notiz, Händler-Historie, Kategorie). Tastatur und Maus; Textgröße in
  Einstellungen › Allgemein › Schriftgröße (×1 / ×1,15 / ×1,3).
- **iOS:** Navigationsstapel; der Inspektor öffnet sich auf dem iPhone als Sheet. Dynamic Type.
- **Bankabruf:** TAN-Freigabe in der App der Bank; Abrufe können Minuten dauern und scheitern
  (gesperrter Zugang, fehlende Freigabe). Fehlermeldungen müssen dann sagen, was zu tun ist.
- **„Problem melden“:** erstellt einen Screenshot der App, in dem persönliche Daten und Beträge
  geschwärzt sind. Jede Ansicht muss dafür markieren, was sensibel ist.
- **Demo-Modus:** `--demo-seed` startet mit fiktiven Daten; alle Screenshots für Doku und Audits entstehen damit.

## Capabilities and Constraints

- Kontoarten: Giro, Spar, Festgeld, Kreditkarte, Zahlungsdienst, Depot, Bargeld, Kredit; dazu Sachwerte
  und Bestellkonten von Online-Händlern. Kategorien, Regeln, Tags, Budgets, Belege, Notizen.
- **Mehrere Währungen:** Beträge verschiedener Währungen werden nie zu einer Summe addiert. EUR ist die
  Hauptwährung; andere Währungen erscheinen getrennt („Zusätzlich: …“).
- **Datenschutz in der Oberfläche:** persönliche Angaben und Beträge werden in „Problem melden“ und im
  Privatsphäre-Modus geschwärzt. Wie Ansichten das markieren: `CLAUDE.md` › Sensitive UI.
- **Keine Fremdmarken:** sichtbare Texte nennen keine anderen Banking- oder Finanzprogramme.
- **Sprache:** Deutsch ist die Hauptsprache der App; Übersetzungen (u. a. Englisch) stehen im String-Katalog `Localizable.xcstrings`.
- **Offen:** einheitliche Abstände und Radien über alle Ansichten (siehe `DESIGN.md` › Shapes).

## Brand Commitments

- Name „GiroCrab“, Krabben-Logo (`GiroCrabApp/Xcode/App/Assets.xcassets/AppIcon.appiconset`).
- In der App siezt GiroCrab die Nutzer („Wählen Sie eine Buchung …“); README und Website duzen.
- Ton: sachlich, kurz, hilfsbereit; keine Werbesprache.

## Evidence on Hand

- Screenshots mit Demo-Daten: `docs/screenshots/`.
- Design-Seiten einzelner Bereiche: `docs/*_UI.md`.
- Keine Nutzerstudien, Testimonials oder Kennzahlen; solche dürfen nicht erfunden werden.

## Product Principles

1. **Ein Arbeitswerkzeug, keine Bühne.** Information vor Dekoration; Farbe trägt Bedeutung (Zufluss, Abfluss, Warnung).
2. **Beträge sind das Wichtigste.** Sie sind immer lesbar, vergleichbar ausgerichtet und werden nie abgeschnitten.
3. **Ruhig bei Fehlern.** Was schiefging und was der nächste Schritt ist, steht in normaler Sprache; technische Codes nur zusätzlich.
4. **Privat ist der Normalfall.** Jede neue Ansicht markiert sensible Inhalte von Anfang an.
5. **Nativ statt gleich.** Jede Plattform fühlt sich wie ihre eigene App an.

## Accessibility & Inclusion

- Textfarben erreichen mindestens 4,5:1 auf ihren Flächen, auch im Modus „Kontrast erhöhen“
  (`GiroCrabApp/Tests/GiroCrabUITests/ThemeContrastTests.swift`).
- Text skaliert: macOS über die eigene Schriftgröße, iOS über Dynamic Type (Regeln: `CLAUDE.md` › Text size).
- Hell- und Dunkelmodus sind gleichwertig; beide werden geprüft.
- Mit „Bewegung reduzieren“ gleitet und federt nichts (`Motion` in `Formatting.swift`).
- Deutsche Texte sind lang: Ansichten bleiben bei schmalen Fenstern und auf dem iPhone lesbar.
- Trefferflächen auf Touch-Geräten mindestens 44 pt (iOS) bzw. 48 dp (Android).
- VoiceOver-Beschriftungen für Bedienelemente ohne sichtbaren Text.
