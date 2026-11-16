import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// SettingsView.swift › GeneralSettings. Mitteilungen, Saldoverlauf (bars) and Händler-Logos
// are not on Linux yet (no announcement feed, line chart only, no online logos).
ScrollView {
    ScrollBar.vertical: ScrollViewBar {}
    id: pane
    contentWidth: availableWidth
    /// LanguagePreference: { choice: "system" | "de" | "en", system, running }.
    property var language: AppModel.run(() => AppModel.core("language_state")) || ({ choice: "system", system: "de", running: "de" })
    /// AppLanguage.nativeName: language names in their own language, as in the system settings.
    function nativeName(code) { return ({ de: "Deutsch", en: "English" })[code] || code }
    readonly property string effectiveLanguage: language.choice === "system" ? language.system : language.choice
    ColumnLayout {
        width: pane.availableWidth - 40
        x: 20
        spacing: 18
        Item { implicitHeight: 2 }
        // LanguagePreference.swift › LanguageSettingsSection
        SettingsFormSection {
            footer: qsTr("„Systemsprache“ folgt der Sprache Ihrer Arbeitsumgebung (z. B. Systemeinstellungen › Region & Sprache).")
            SettingsFormRow {
                label: qsTr("Sprache")
                AppComboBox {
                    Layout.preferredWidth: 200 * Theme.textScale
                    model: [{ id: "system", title: qsTr("Systemsprache (%1)").arg(pane.nativeName(pane.language.system)) },
                            { id: "de", title: pane.nativeName("de") }, { id: "en", title: pane.nativeName("en") }]
                    textRole: "title"; valueRole: "id"
                    currentIndex: ["system", "de", "en"].indexOf(pane.language.choice)
                    onActivated: {
                        AppModel.run(() => AppModel.core("set_language_choice", { choice: currentValue }))
                        pane.language = AppModel.run(() => AppModel.core("language_state")) || pane.language
                    }
                }
            }
            SettingsFormRow {
                visible: pane.effectiveLanguage !== pane.language.running
                label: qsTr("Die neue Sprache wird nach einem Neustart verwendet.")
                Button {
                    text: qsTr("Neu starten")
                    onClicked: { AppModel.run(() => AppModel.core("request_relaunch")); Qt.quit() }
                }
            }
        }
        SettingsFormSection {
            title: qsTr("Ansicht")
            // Appearance.swift › AppAppearance (GitHub #388)
            SettingsFormRow {
                label: qsTr("Erscheinungsbild")
                AppComboBox {
                    Layout.preferredWidth: 200 * Theme.textScale
                    model: [{ id: "system", title: qsTr("System") }, { id: "light", title: qsTr("Hell") }, { id: "dark", title: qsTr("Dunkel") }]
                    textRole: "title"; valueRole: "id"
                    currentIndex: Math.max(0, ["system", "light", "dark"].indexOf(Theme.appearanceSettings.appearance))
                    onActivated: Theme.appearanceSettings.appearance = currentValue
                }
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   text: qsTr("Hell oder Dunkel gilt nur für GiroCrab; „System“ folgt der Einstellung des Systems.") }
            // BookingDensity.swift (#542)
            SettingsFormRow {
                label: qsTr("Buchungsdichte")
                AppComboBox {
                    Layout.preferredWidth: 200 * Theme.textScale
                    model: BookingPrefs.densities
                    textRole: "title"; valueRole: "id"
                    currentIndex: BookingPrefs.densities.findIndex(d => d.id === BookingPrefs.density)
                    onActivated: BookingPrefs.setDensity(currentValue)
                }
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   text: qsTr("Kompakt öffnet die aktive Buchung mit allen Details. Standard zeigt einen kurzen Verwendungszweck. Großzügig zeigt alle Buchungen ausführlich.") }
            // TextSize.swift (#343): Schriftgröße
            SettingsFormRow {
                label: qsTr("Schriftgröße")
                AppComboBox {
                    Layout.preferredWidth: 200 * Theme.textScale
                    model: Theme.textSizes
                    textRole: "title"; valueRole: "id"
                    currentIndex: Theme.textSizes.findIndex(s => s.id === Theme.textSize)
                    onActivated: Theme.setTextSize(currentValue)
                }
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   text: qsTr("Vergrößert die Schrift in der ganzen App. Auch im Menü Darstellung mit %1 / %2 / %3.")
                             .arg(Fmt.keys("Strg+Plus")).arg(Fmt.keys("Strg+Minus")).arg(Fmt.keys("Strg+0")) }
            CheckBox {
                text: qsTr("Auswertungen in der Übersicht anzeigen")
                checked: AppModel.showStatistics
                onToggled: AppModel.showStatistics = checked
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   text: qsTr("Blendet Saldoverlauf, Cashflow und Budgets über der Buchungsliste aus. Auch im Menü Ansicht umschaltbar.") }
            CheckBox {
                text: qsTr("Budgets anzeigen")
                checked: AppModel.showBudgets
                onToggled: AppModel.showBudgets = checked
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   text: qsTr("Wer keine Monatsbudgets nutzt, blendet die Budget-Karte und „Monatsbudget festlegen …“ ganz aus. Auch im Menü Ansicht umschaltbar.") }
            // NegativeAmountStyle.swift (#218, #571)
            SettingsFormRow {
                label: qsTr("Negative Beträge")
                AppComboBox {
                    Layout.preferredWidth: 200 * Theme.textScale
                    model: BookingPrefs.negativeStyles
                    textRole: "title"; valueRole: "id"
                    currentIndex: BookingPrefs.negativeStyles.findIndex(s => s.id === BookingPrefs.negativeStyle)
                    onActivated: BookingPrefs.setNegativeStyle(currentValue)
                }
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   text: qsTr("Farbe von Abbuchungen in der Buchungsliste: neutral wie der übrige Text, rot wie negative Salden in der Seitenleiste oder kontraststark (orange und blau, auch bei Rot-Grün-Schwäche gut unterscheidbar). Das Vorzeichen bleibt immer sichtbar.") }
        }
        SettingsFormSection {
            title: qsTr("Buchungen")
            SettingsFormRow {
                label: qsTr("Als geprüft markieren")
                AppComboBox {
                    Layout.preferredWidth: 200 * Theme.textScale
                    model: AppModel.autoReviewDelays
                    textRole: "title"; valueRole: "id"
                    currentIndex: AppModel.autoReviewDelays.findIndex(d => d.id === AppModel.autoReviewDelay)
                    onActivated: AppModel.autoReviewDelay = currentValue
                }
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   text: qsTr("Eine ausgewählte Buchung gilt nach dieser Zeit als geprüft – wie eine gelesene E-Mail. Bei Mehrfachauswahl nie.") }
        }
        Item { implicitHeight: 8 }
    }
}
