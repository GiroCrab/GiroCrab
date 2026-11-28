import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// MerchantRecognitionSettings.swift › CustomMerchantEditor + MerchantDraft (#1373): name and search
// words of an own merchant – new, changed, or a copy of a bundled one. The core checks and
// normalises the search words; its message stays under the fields and the sheet stays open.
// Layout and rules: docs/HAENDLER_UI.md.
Sheet {
    id: sheet
    /// The own merchant changed; null creates one.
    property var uuid: null
    /// Key of the bundled merchant a new one copies.
    property var basedOn: null
    /// Its name, for the title and the hint.
    property var basedOnName: null
    property string error: ""
    property bool saving: false
    /// Runs after a successful save, before the sheet closes (the settings list reloads).
    signal saved()
    sheetWidth: 460

    readonly property bool canSave: !saving && nameField.text.trim() !== ""
                                    && AppModel.merchantNeedles(needlesField.text).length > 0

    function present(name, needles) {
        nameField.text = name
        needlesField.text = needles
        error = ""
        saving = false
        open()
        nameField.forceActiveFocus()
    }
    /// MerchantDraft()
    function create() { uuid = null; basedOn = null; basedOnName = null; present("", "") }
    /// MerchantDraft(_ m: CustomMerchant)
    function edit(m) { uuid = m.uuid; basedOn = null; basedOnName = null; present(m.name, AppModel.merchantNeedlesText(m.needles)) }
    /// MerchantDraft(copying: BuiltinMerchant)
    function copy(m) { uuid = null; basedOn = m.key; basedOnName = m.name; present(m.name, AppModel.merchantNeedlesText(m.needles)) }

    function save() {
        if (!canSave) return
        saving = true
        error = ""
        AppModel.saveCustomMerchant(uuid, nameField.text, AppModel.merchantNeedles(needlesField.text),
                                    uuid === null ? basedOn : null, () => {
            saving = false
            sheet.saved()
            close()
        }, message => {
            saving = false
            error = message
        })
    }

    ColumnLayout {
        width: sheet.availableWidth
        spacing: 14
        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: sheet.uuid !== null ? qsTr("Händler bearbeiten")
                  : sheet.basedOnName !== null ? qsTr("„%1“ als eigenen Händler").arg(sheet.basedOnName)
                  : qsTr("Eigener Händler")
            font.pixelSize: Theme.title3; font.weight: Font.Bold; color: Theme.text
        }
        GridLayout {
            Layout.fillWidth: true
            columns: 2; columnSpacing: 12; rowSpacing: 10
            Text { text: qsTr("Name"); font.pixelSize: Theme.body; color: Theme.text }
            TextField {
                id: nameField
                Layout.fillWidth: true
                onAccepted: sheet.save()
                Sensitive { when: parent.text !== "" }
            }
            Text { text: qsTr("Suchwörter"); font.pixelSize: Theme.body; color: Theme.text; Layout.alignment: Qt.AlignTop; topPadding: 6 }
            TextArea {
                id: needlesField
                Layout.fillWidth: true
                Layout.minimumHeight: 2 * font.pixelSize * 1.6
                Layout.maximumHeight: 5 * font.pixelSize * 1.6
                wrapMode: TextEdit.Wrap
                placeholderText: qsTr("z. B. muster gmbh, muster shop")
                inputMethodHints: Qt.ImhNoPredictiveText
                Sensitive { when: parent.text !== "" }
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   text: qsTr("Mehrere Suchwörter mit Komma trennen; jedes passt nur als ganzes Wort. Gesucht wird zuerst in der Gegenseite, ohne Gegenseite im Verwendungszweck, bei Zahlungsdiensten auch dahinter (etwa „Ihr Einkauf bei …“).") }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   visible: sheet.basedOnName !== null
                   text: sheet.basedOnName !== null ? qsTr("Die Kopie behält Logo, Farbe und Art von „%1“ und ersetzt den eingebauten Händler, solange es sie gibt.").arg(sheet.basedOnName) : "" }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.warning
                   visible: sheet.error !== ""; text: sheet.error }
        }
        RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            Button { text: qsTr("Abbrechen"); onClicked: sheet.close() }
            Button {
                text: sheet.uuid === null ? qsTr("Anlegen") : qsTr("Sichern")
                highlighted: true
                enabled: sheet.canSave
                onClicked: sheet.save()
            }
        }
    }
}
