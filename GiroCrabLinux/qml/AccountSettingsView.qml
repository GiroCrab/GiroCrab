import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// AccountSettingsView.swift: "Einstellungen …" of an account (sidebar context menu, #981),
// design page docs/KONTO_EINSTELLUNGEN_UI.md. One sheet, built like the former "Konto
// bearbeiten" dialog, that replaces "Konto bearbeiten …", "Umbenennen …", "Symbol & Farbe …",
// "Kontoart ändern …", "Kontoinhaber …" and "Startsaldo …". Changes are applied with Sichern;
// only Symbol & Farbe applies at once, like its popover always did. The note is a field of the
// account, synced like its name (#980).
Sheet {
    id: sheet
    property var accountID: null
    readonly property var account: accountID === null ? null : (AppModel.accountsByID[accountID] || null)
    /// The account when the sheet opened: what a sync changes meanwhile is no edit here (#980).
    property var original: null
    /// an offline account is always cash (#679): no Kontoart
    readonly property bool offline: accountID !== null && OfflineAccounts.isOffline(accountID)
    /// The Startsaldo only exists for accounts whose balance is computed from the bookings.
    property bool showsOpeningBalance: false
    readonly property var openingCents: Fmt.parseCents(openingField.text)
    readonly property bool openingValid: openingField.text.trim() === "" || openingCents !== null
    readonly property bool canSave: nameField.text.trim() !== "" && (!showsOpeningBalance || openingValid)
    sheetWidth: 440
    onClosed: { accountID = null; if (AppModel.iconColorTarget && AppModel.iconColorTarget.host === "settings") AppModel.iconColorTarget = null }

    function edit(id) {
        const a = AppModel.accountsByID[id]
        if (!a) return
        accountID = id
        original = a
        nameField.text = a.name
        ownerField.text = a.owner || ""
        kindBox.currentIndex = Math.max(0, AppModel.accountKindChoices.findIndex(k => k.id === a.kind))
        showsOpeningBalance = AppModel.canSetOpeningBalance(a)
        openingField.text = a.opening_balance_cents !== null && a.opening_balance_cents !== undefined ? Fmt.plain(a.opening_balance_cents) : ""
        noteField.text = a.note || ""
        archivedBox.checked = a.archived_unix !== null
        open()
        nameField.forceActiveFocus()
    }

    function save() {
        const a = original
        if (!a || !account || !canSave) return
        const newName = nameField.text.trim()
        const owner = ownerField.text.trim()
        const kind = AppModel.accountKindChoices[kindBox.currentIndex].id
        const note = noteField.text.trim()
        if (newName !== a.name) AppModel.renameAccount(a.id, newName)
        if (owner !== (a.owner || "")) AppModel.setAccountOwner(a.id, owner)
        if (kind !== a.kind && !offline) AppModel.setAccountKind(a.id, kind)
        if (note !== (a.note || "")) AppModel.setAccountNote(a.id, note)
        if (showsOpeningBalance) {
            const text = openingField.text.trim()
            const stored = a.opening_balance_cents === undefined ? null : a.opening_balance_cents
            try {
                if (text === "") {
                    if (stored !== null) AppModel.setOpeningBalance(a.id, null, "")
                } else if (openingCents !== null && openingCents !== stored) {
                    AppModel.setOpeningBalance(a.id, openingCents, a.opening_balance_date || "")
                }
            } catch (e) {
                AppModel.errorMessage = AppModel.parseCoreError(e.message).message
                return
            }
        }
        // last: archiving reloads everything
        const archived = archivedBox.checked
        close()
        if (archived !== (a.archived_unix !== null)) AppModel.setAccountArchived(a, archived)
    }

    /// The Stichtag sheet is its own sheet: this one makes room first; unsaved input is lost.
    function openCalibration() {
        const a = account
        if (!a) return
        close()
        AppModel.openOpeningBalance(a)
    }

    ColumnLayout {
        width: sheet.availableWidth
        spacing: 14
        Text { text: qsTr("Kontoeinstellungen"); font.pixelSize: Theme.title3; font.weight: Font.Bold; color: Theme.text }
        GridLayout {
            Layout.fillWidth: true
            columns: 2
            columnSpacing: 12; rowSpacing: 8
            // Kontoname with the Symbol & Farbe button (applies at once)
            FieldLabel { text: qsTr("Kontoname") }
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                Rectangle {
                    id: symbolButton
                    implicitWidth: signetRow.implicitWidth + 12; implicitHeight: Math.max(26, signetRow.implicitHeight + 6)
                    radius: 6
                    color: symbolHover.hovered ? Theme.withAlpha(Theme.secondary, 0.18) : Theme.withAlpha(Theme.secondary, 0.1)
                    border.color: Theme.border
                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("Symbol & Farbe …")
                    ToolTip.visible: symbolHover.hovered; ToolTip.delay: 600; ToolTip.text: qsTr("Symbol & Farbe …")
                    HoverHandler { id: symbolHover }
                    RowLayout {
                        id: signetRow
                        anchors.centerIn: parent
                        spacing: 2
                        AccountSignet { account: sheet.account; size: 20
                                        tintColor: sheet.account ? AppModel.accountColor(sheet.account.id) : Theme.secondary }
                        SFIcon { symbol: "chevron.down"; size: 8; color: Theme.secondary }
                    }
                    TapHandler {
                        onTapped: {
                            const p = symbolButton.mapToItem(null, 0, symbolButton.height + 6)
                            iconPopover.x = Math.max(8, Math.min(p.x, (Overlay.overlay ? Overlay.overlay.width : 1000) - iconPopover.width - 8))
                            iconPopover.y = Math.max(8, Math.min(p.y, (Overlay.overlay ? Overlay.overlay.height : 800) - 560))
                            AppModel.iconColorTarget = { type: "account", id: sheet.accountID, host: "settings" }
                        }
                    }
                }
                TextField { id: nameField; Layout.fillWidth: true; onAccepted: sheet.save(); Sensitive {} }
            }
            FieldLabel { text: qsTr("Kontoinhaber") }
            TextField { id: ownerField; Layout.fillWidth: true; onAccepted: sheet.save(); Sensitive {} }
            // an offline account is always cash (#679)
            FieldLabel { visible: !sheet.offline; text: qsTr("Kontoart") }
            AppComboBox { id: kindBox; visible: !sheet.offline
                          Layout.fillWidth: true; model: AppModel.accountKindChoices; textRole: "text" }
            FieldLabel { visible: sheet.showsOpeningBalance; text: qsTr("Startsaldo") }
            RowLayout {
                visible: sheet.showsOpeningBalance
                Layout.fillWidth: true
                spacing: 6
                TextField {
                    id: openingField
                    Layout.fillWidth: true
                    horizontalAlignment: TextInput.AlignRight; font.features: { "tnum": 1 }
                    placeholderText: qsTr("z. B. %1").arg(Fmt.plain(125000))
                    color: sheet.openingValid ? Theme.text : Theme.negative
                    onAccepted: sheet.save()
                    Sensitive { amounts: true }
                }
                Text { text: sheet.account ? sheet.account.currency : ""; font.pixelSize: Theme.body; color: Theme.secondary }
            }
            Item { visible: sheet.showsOpeningBalance; implicitWidth: 1; implicitHeight: 1 }
            Text {
                visible: sheet.showsOpeningBalance
                text: qsTr("Stichtag und Kalibrierung …"); font.pixelSize: Theme.body; color: Theme.accent
                HoverHandler { id: calibrationHover; cursorShape: Qt.PointingHandCursor }
                ToolTip.visible: calibrationHover.hovered; ToolTip.delay: 600
                ToolTip.text: qsTr("Startsaldo ab einem Stichtag oder passend zu einem Kontostand eintragen")
                TapHandler { onTapped: sheet.openCalibration() }
            }
            // Kontonummer, IBAN, Bank: only shown with a value; they come from the bank or the import
            FieldLabel { visible: numberValue.text !== ""; text: qsTr("Kontonummer") }
            ReadOnlyValue { id: numberValue; text: sheet.account ? sheet.account.account_number || "" : "" }
            FieldLabel { visible: ibanValue.text !== ""; text: qsTr("IBAN") }
            ReadOnlyValue { id: ibanValue; text: sheet.account ? sheet.account.iban || "" : "" }
            FieldLabel { visible: bankValue.text !== ""; text: qsTr("Bank") }
            ReadOnlyValue { id: bankValue; text: sheet.account ? sheet.account.bank_name || "" : "" }
            // Konto-Notiz: the label next to the editor's top edge
            FieldLabel { text: qsTr("Konto-Notiz"); Layout.alignment: Qt.AlignRight | Qt.AlignTop; Layout.topMargin: 6 }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 4
                ScrollView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 80
                    ScrollBar.vertical: ScrollViewBar {}
                    TextArea {
                        id: noteField
                        wrapMode: TextEdit.Wrap
                        font.pixelSize: Theme.body
                        background: Rectangle { radius: 6; color: "transparent"; border.color: Theme.withAlpha(Theme.tertiary, 0.5) }
                        Sensitive {}
                    }
                }
                Text { text: qsTr("Erscheint unter dem Kontonamen in der Kontoansicht."); font.pixelSize: Theme.caption; color: Theme.tertiary
                       wrapMode: Text.WordWrap; Layout.fillWidth: true }
            }
            Item { implicitWidth: 1; implicitHeight: 1 }
            CheckBox { id: archivedBox; text: qsTr("Das Konto wurde aufgelöst") }
        }
        RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            Button { text: qsTr("Abbrechen"); onClicked: sheet.close() }
            Button { text: qsTr("Sichern"); highlighted: true; enabled: sheet.canSave; onClicked: sheet.save() }
        }
    }

    // the "Symbol & Farbe" popover of this sheet (capturablePopover), over the sheet; modal
    // without dimming and with the focus, so Esc closes only the popover, not the sheet with
    // its unsaved input
    IconColorPopover { id: iconPopover; host: "settings"; parent: Overlay.overlay; z: 20; modal: true; dim: false; focus: true }

    component FieldLabel: Text {
        font.pixelSize: Theme.body; color: Theme.text
        Layout.alignment: Qt.AlignRight | Qt.AlignVCenter
    }
    /// LabeledContent(label) { Text(verbatim: value).textSelection(.enabled) }: shown only with a value,
    /// each value can be selected and copied on its own.
    component ReadOnlyValue: TextEdit {
        visible: text !== ""
        readOnly: true; selectByMouse: true
        font.pixelSize: Theme.body; color: Theme.text
        Layout.fillWidth: true
        Sensitive {}
    }
}
