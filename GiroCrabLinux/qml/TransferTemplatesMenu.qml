import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// PlanningUI.swift › TransferTemplatesMenu: „Vorlagen“ in the header of the transfer and the
// standing order form (#1475): apply a template, delete one, save the form's recipient as one.
// `standing`: applies to and saves from the standing order's draft (Payments.standingDraft).
Button {
    id: root
    property bool standing: false
    property var templates: []
    /// The name or delete dialog is open: the form's Return / Esc shortcuts stand back.
    readonly property bool dialogOpen: templateName.visible || deleteConfirm.visible
    readonly property var draft: standing ? Payments.standingDraft : AppModel.transferDraft

    flat: true; icon.name: SF.icon("doc.on.doc"); text: qsTr("Vorlagen")
    // .task { templates = … }: read when the menu opens
    onClicked: { templates = AppModel.transferTemplates(); templatesMenu.popup(root, 0, root.height) }

    AppMenu {
        id: templatesMenu
        // `if !templates.isEmpty { Menu("Vorlage löschen") }`: the submenu's entry
        // is hidden while there are no templates.
        onAboutToShow: {
            for (let i = 0; i < count; ++i) {
                const item = itemAt(i)
                if (item && item.subMenu === deleteMenu) {
                    item.visible = root.templates.length > 0
                    item.height = item.visible ? item.implicitHeight : 0
                }
            }
        }
        Instantiator {
            model: root.templates
            delegate: AppMenuItem {
                required property var modelData
                text: modelData.name + " · " + modelData.recipient_name
                // leaves Umbuchung; from a Tagesgeld account only amount and purpose (fillTransferDraft, #881)
                onTriggered: AppModel.applyTransferTemplate(modelData, root.standing)
            }
            onObjectAdded: (i, o) => templatesMenu.insertItem(i, o)
            onObjectRemoved: (i, o) => templatesMenu.removeItem(o)
        }
        AppMenuSeparator { visible: root.templates.length > 0; height: visible ? implicitHeight : 0 }
        AppMenu {
            id: deleteMenu
            title: qsTr("Vorlage löschen")
            enabled: root.templates.length > 0
            Instantiator {
                model: root.templates
                delegate: AppMenuItem {
                    required property var modelData
                    text: modelData.name + " …"
                    onTriggered: { deleteConfirm.template = modelData; deleteConfirm.open() }
                }
                onObjectAdded: (i, o) => deleteMenu.insertItem(i, o)
                onObjectRemoved: (i, o) => deleteMenu.removeItem(o)
            }
        }
        AppMenuItem {
            text: qsTr("Als Vorlage speichern …"); icon.name: SF.icon("square.and.arrow.down")
            enabled: root.draft.name.trim() !== "" && root.draft.iban !== ""
            onTriggered: { templateNameField.text = root.draft.name; templateName.open() }
        }
    }

    // .alert("Vorlage speichern") { TextField("Name der Vorlage") }
    Sheet {
        id: templateName
        reportCapturable: false   // an alert on the Mac (ReportDialogs.qml)
        sheetWidth: 340
        ColumnLayout {
            width: templateName.availableWidth
            spacing: 12
            Text { text: qsTr("Vorlage speichern"); font.pixelSize: Theme.body + 2; font.weight: Font.Bold; color: Theme.text }
            TextField { id: templateNameField; Layout.fillWidth: true; placeholderText: qsTr("Name der Vorlage"); onAccepted: saveButton.clicked() }
            RowLayout {
                Item { Layout.fillWidth: true }
                Button { text: qsTr("Abbrechen"); onClicked: templateName.close() }
                Button {
                    id: saveButton
                    text: qsTr("Speichern"); highlighted: true
                    onClicked: { AppModel.saveTransferTemplate(templateNameField.text, root.standing); root.templates = AppModel.transferTemplates(); templateName.close() }
                }
            }
        }
    }

    // .confirmationDialog("Vorlage „…“ löschen?")
    Sheet {
        id: deleteConfirm
        reportCapturable: false   // an alert on the Mac (ReportDialogs.qml)
        property var template: null
        sheetWidth: 340
        ColumnLayout {
            width: deleteConfirm.availableWidth
            spacing: 12
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap
                   text: qsTr("Vorlage „%1“ löschen?").arg(deleteConfirm.template ? deleteConfirm.template.name : "")
                   font.pixelSize: Theme.body + 2; font.weight: Font.Bold; color: Theme.text }
            RowLayout {
                Item { Layout.fillWidth: true }
                Button { text: qsTr("Abbrechen"); onClicked: deleteConfirm.close() }
                Button {
                    text: qsTr("Löschen")
                    palette.buttonText: Theme.negative
                    onClicked: { AppModel.deleteTransferTemplate(deleteConfirm.template.id); root.templates = AppModel.transferTemplates(); deleteConfirm.close() }
                }
            }
        }
    }
}
