# Übersicht der UI-Audits

Stand 25.11.2026, SwiftUI auf macOS, iPhone und iPad, `main` @ `09f11987`. Jede Zeile benennt den
Umfang für einen eigenen Bericht nach der [Audit-Anleitung](../ui-audit.md). Die Reihenfolge startet mit
häufig genutzten Ansichten und Abläufen, bei denen Schriftgröße, Fokus oder Schwärzung besonders wichtig sind.

## Begonnene Audits

| Bericht | Stand | Nächste Prüfung |
|---|---|---|
| [Buchungsinspektor](buchungs-inspektor.md) | Neun Befunde behoben. INS-12 und INS-13 offen, INS-6 ist eine gemeinsame Designentscheidung. | Notizfeld im Dunkelmodus, Zugriff auf Händler-Historie und Kategorisieren mehrerer Buchungen per Maus und Tastatur. |
| [Vermögen](vermoegen.md) | VER-1 bis VER-4 über [#1861](https://github.com/GiroCrab/GiroCrab/pull/1861) behoben. VER-5 und VER-6 betreffen Ringdiagramm und Legende bei großer Schrift. | Ringsumme und Legendenbeträge nachprüfen. Dieselbe Komponente in Depot und Sachwerte-Konto prüfen. [#998](https://github.com/GiroCrab/GiroCrab/issues/998) bleibt für den dort beschriebenen Datenfall offen. |
| [Buchungsliste](buchungsliste.md) | BUC-1 bis BUC-3 dokumentiert. Betrifft Kategorienamen, Direktwahl und Kontrast. | Tastaturfokus und Kategorie-Popover auf dem Mac prüfen. |

## Fehlende Berichte für die Hauptansichten

| Ansicht | Einstieg in `GiroCrabUI/` | Umfang |
|---|---|---|
| Seitenleiste | `SidebarView.swift`, `AllAccountsRoot.swift` | Konten- und Kategorienbaum, Salden, kompakte Darstellung und Navigation per Tastatur. Fokus-Fix in [#1883](https://github.com/GiroCrab/GiroCrab/pull/1883). |
| Buchungssuche und Filter | `BookingListView.swift`, `BalanceHeaderView.swift`, `TagFilterUI.swift` | Suche, Zeitraum, Betrag, Tags und der Zustand ohne Treffer. |
| Kontoübersicht und Kennzahlen | `BalanceHeaderView.swift`, `BookingListView.swift` | „Alle Konten“, Kontenkopf, Kennzahl-Leiste und Saldoverlauf. Beträge in schmalen Fenstern und mit großer Schrift prüfen. |
| Neue Umsätze | `NewBookingsView.swift` | Gruppierung, Leerzustand und Abarbeiten ungelesener Buchungen. |
| Sparpläne & Perks | `InvestmentDashboardView.swift`, `BookingListView.swift` | Gruppierung der Anlagebuchungen, Kennzahlen und Navigation zu Buchungen. |
| Belege & Postbox | `DocumentsUI.swift`, `AccountDocumentsUI.swift` | Liste, Vorschau und Schwärzung von Dokumenten. iPhone-Layout-Fix in [#1895](https://github.com/GiroCrab/GiroCrab/pull/1895). |
| Geplant | `PlannedPaymentsView.swift` | Liste und Detail für vorgemerkte Zahlungen und Daueraufträge. |
| Lastschriften | `DirectDebitsView.swift`, `MandateInspector.swift` | Mandatsliste, Status und Details. |
| Abos & Verträge | `SubscriptionsView.swift`, `SubscriptionInspector.swift` | Liste, Filter, Vertragsdetails und Bearbeitung. Die Beschreibung steht in [ABOS_UI.md](../ABOS_UI.md). |
| Einzeldepot | `DepotDetailView.swift`, `DepotPositionsTable.swift` | Positionen, Historie, Aktivitäten und Sparpläne. Diagrammsteuerung aus VER-1 mitprüfen. |
| Sachwerte-Konto | `TangibleAccountDetailView.swift`, `TangibleAssetsUI.swift` | Positionen, Werte, Mengen und gemeinsame Diagrammsteuerung aus VER-1. |

## Fehlende Berichte für Dialoge und Einstellungen

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
