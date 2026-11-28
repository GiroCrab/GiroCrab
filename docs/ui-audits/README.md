# Übersicht der UI-Audits

Letzter Abgleich am 25.11.2026. Die 14 Hauptansichten auf macOS, iPhone und iPad sind in fünf Themenberichten
dokumentiert. Jede Ansicht hat dort ihren eigenen Prüfstand und Prüfnachweis. Die [Audit-Anleitung](../ui-audit.md)
beschreibt den Ablauf.

## Hauptansichten

| Ansicht und Prüfnachweis | Stand und nächste Prüfung |
|---|---|
| [Seitenleiste](seitenleiste.md#seitenleiste) | SEI-1 und SEI-2 behoben und bei großer Schrift nachgeprüft. Pfeiltasten-Fokus in [#1883](https://github.com/GiroCrab/GiroCrab/pull/1883). |
| [Buchungsliste](buchungen.md#buchungsliste) | BUC-1 bis BUC-3 offen. Kategorienamen, Direktwahl und Kontrast prüfen. Tastaturfokus und Kategorie-Popover auf dem Mac stehen aus. |
| [Buchungsinspektor](buchungen.md#buchungsinspektor) | Neun Befunde behoben. INS-12 und INS-13 offen. INS-6 ist eine gemeinsame Designentscheidung. |
| [Buchungssuche und Filter](buchungen.md#buchungssuche-und-filter) | SUC-1 offen. Weitere Filter bei großer Schrift auffindbar machen. |
| [Kontoübersicht und Kennzahlen](buchungen.md#kontoübersicht-und-kennzahlen) | KON-1 bis KON-3 offen. Beträge und Achsenwerte bei großer Schrift prüfen. |
| [Neue Umsätze](buchungen.md#neue-umsätze) | NEU-1 in [#2046](https://github.com/GiroCrab/GiroCrab/pull/2046) behoben und nachgeprüft. Kontoname und Saldo sind bei großer Schrift lesbar. |
| [Belege & Postbox](belege-und-postbox.md#belege--postbox) | BEL-1 nach [#1895](https://github.com/GiroCrab/GiroCrab/pull/1895) als `fixed` bestätigt. BEL-2 zu Titel und Datum in den Dokumentzeilen ist in [#2021](https://github.com/GiroCrab/GiroCrab/pull/2021) behoben und nachgeprüft. |
| [Geplant](zahlungen-und-vertraege.md#geplant) | GPL-1 und GPL-2 offen. Filter, Summen und Detailtitel auf dem iPhone prüfen. |
| [Lastschriften](zahlungen-und-vertraege.md#lastschriften) | LAS-1 offen. Mandatsliste bei großer Schrift erreichbar halten. Der Detailtitel ist in GPL-2 erfasst. |
| [Abos & Verträge](zahlungen-und-vertraege.md#abos--verträge) | ABO-1 und ABO-2 offen. iPhone-Breite und Öffnen des Vertragsdetails prüfen. |
| [Vermögen](vermoegen-und-anlagen.md#vermögen) | VER-1 bis VER-4 über [#1861](https://github.com/GiroCrab/GiroCrab/pull/1861) behoben. VER-5 und VER-6 offen. Ringsumme und Legende bei großer Schrift prüfen. |
| [Sparpläne & Perks](vermoegen-und-anlagen.md#sparpläne--perks) | SPA-1 offen. Planraster bei großer Schrift lesbar halten. Für Saveback und Round up fehlt ein Demo-Fall. |
| [Einzeldepot](vermoegen-und-anlagen.md#einzeldepot) | DEP-1 offen. Kopf und Wertverlauf bei großer Schrift zusammen mit SAC-1 prüfen. |
| [Sachwerte-Konto](vermoegen-und-anlagen.md#sachwerte-konto) | SAC-1 offen. Gemeinsames Layout mit dem Depot prüfen. |

## Prüfbuilds

- **iOS am 25.11.2026:** Für den jeweils angegebenen App-Commit wurden die Swift-Dateien aus
  [e7f26fd6ca4a2b7697b4afdc120e62ba1c16489d](https://github.com/GiroCrab/GiroCrab/commit/e7f26fd6ca4a2b7697b4afdc120e62ba1c16489d)
  in einer Prüfkopie angewendet. Der Commit gehört zu [#1919](https://github.com/GiroCrab/GiroCrab/pull/1919).
- **Core der ursprünglichen Aufnahmen vom 25.11.2026:** Das wiederverwendete xcframework meldet
  `v0.8.8-746-ga63b7c31-dirty` (`release`, macOS und iOS). Der Core-Fingerabdruck weicht vom sauberen
  `a63b7c31` ab. Die damaligen lokalen Build-Eingaben sind nur teilweise bekannt. Für neue Aufnahmen den
  Core aus dem dokumentierten App-Commit bauen und Abweichungen direkt beim Prüfnachweis erfassen.

## Weitere Audit-Kandidaten

Dialoge und Einstellungen für spätere Durchgänge.

| Ansicht | Einstieg in `GiroCrabUI/` | Umfang |
|---|---|---|
| Überweisung | `TransferSheetView.swift`, `PayeeCheckSheet.swift` | Eingabe und Empfängerprüfung. iPhone-Layout-Fix in [#1894](https://github.com/GiroCrab/GiroCrab/pull/1894). |
| Dauerauftrag bearbeiten | `StandingOrderSheet.swift` | Rhythmus, Termin, Zusammenfassung und Bestätigung. |
| Sammelüberweisung | `BatchTransferSheet.swift` | Einzelposten, Gesamtsumme und Bestätigung. |
| Lastschrift einziehen | `CollectionSheet.swift`, `MandateEditorSheet.swift` | Mandat wählen oder bearbeiten, Einzug zusammenfassen. |
| Bank hinzufügen | `AddBankAssistant.swift`, `BankPicker.swift`, `Onboarding.swift` | Bank finden, Zugangsart wählen und ersten Abruf starten. |
| Abruf, TAN und Fehler | `TwoFactorSheet.swift`, `SyncLog.swift`, `ConnectionsUI.swift` | Laufender Abruf, ausstehende Freigabe, Abbruch und verständlicher nächster Schritt bei Fehlern. |
| Kontoeinstellungen | `AccountSettingsView.swift` | Kontoname, Einbeziehung in Summen, Archivieren und Löschen. |
| Kontogruppen | `AccountGroupsUI.swift` | Gruppen bearbeiten, Konten zuordnen und Reihenfolge ändern. |
| Offline-Konten | `OfflineAccountsUI.swift`, `OpeningBalanceUI.swift` | Konto anlegen, Anfangssaldo und manuelle Buchungen erfassen. |
| Buchung aufteilen | `PlanningUI.swift` | Teilbeträge oder Prozente eingeben, Rundung und Restbetrag prüfen. |
| Erstattungen zuordnen | `RefundsUI.swift` | Offene Erstattung finden, zuordnen und die Zuordnung wieder lösen. |
| Kategorien und Regeln | `NewCategoryDialog.swift`, `RuleEditorView.swift`, `CommandPalette.swift` | Kategorie anlegen, Regel bearbeiten und Vorschläge in der Kategorisierung prüfen. |
| Tags verwalten | `TagManagerView.swift`, `RuleTagsUI.swift` | Tags und Tag-Gruppen verwalten und Regeln zuordnen. |
| Budgets und Auswertung | `PlanningUI.swift`, `CategorySumChart.swift` | Budget anlegen, Fortschritt lesen und Zeitraum wechseln. |
| Datenimport | `RootView.swift`, `BackupImportView.swift` | Datei wählen, Vorschau verstehen und auf Fehlermeldungen reagieren. |
| Export und Drucken | `ExportSheet.swift`, `PrintSheet.swift` | Auswahl und Filter nachvollziehen, Dateiformat oder Druckumfang wählen. |
| Tresor und Sicherung | `RootView.swift`, `VaultBackupUI.swift` | Einstieg, Sperrbildschirm, Sicherung und Wiederherstellung. |
| Allgemeine Einstellungen | `SettingsView.swift`, `ListSettings.swift` | Darstellung, Schriftgröße, Listenoptionen und App-Verhalten. |
| Bankzugänge bearbeiten | `SettingsView.swift`, `ConnectionsUI.swift` | Status der Zugänge, Bearbeiten und Entfernen. |
| Weitere Einstellungsreiter | `SettingsView.swift`, `MerchantRecognitionSettings.swift`, `ExternalAccessSettings.swift` | Händler, Dokumente, Sicherheit und Sync, Analyse und Updates. Je Reiter einen eigenen Bericht anlegen. |
| Problem melden | `BugReportUI.swift`, `ReportDialogs.swift` | Vorschau, Schwärzung und Freigabe mit Hauptfenster, Sheet und Einstellungen. |
| Hilfe und Mitteilungen | `HelpView.swift`, `AnnouncementsUI.swift` | Hilfe finden, Mitteilungen erkennen und schließen. |
| Menüleiste (macOS) | `MenuBarView.swift` | Kontostand, Sperrzustand und Wechsel ins Hauptfenster. |
