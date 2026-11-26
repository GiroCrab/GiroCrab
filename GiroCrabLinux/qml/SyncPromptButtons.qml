import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// TwoFactorSheet.swift › SyncPromptButtons: Abbrechen / Alle abbrechen + Überspringen + default.
RowLayout {
    id: buttons
    property string defaultTitle: qsTr("Weiter")
    property bool defaultVisible: true
    property bool defaultEnabled: true
    /// `leading`: extra buttons right after Abbrechen (e.g. „TAN auslassen“, #1827).
    default property alias leading: leadingRow.data
    signal cancel()
    signal accept()
    Button {
        text: AppModel.canSkipInSyncBatch ? qsTr("Alle abbrechen") : qsTr("Abbrechen")
        onClicked: AppModel.canSkipInSyncBatch ? AppModel.cancelAllSync() : buttons.cancel()
    }
    // empty: no width, the filler after it takes the extra spacing
    RowLayout { id: leadingRow }
    Item { Layout.fillWidth: true }
    Button { visible: AppModel.canSkipInSyncBatch; text: qsTr("Überspringen"); onClicked: buttons.cancel() }
    Button { visible: buttons.defaultVisible; text: AppModel.menuText(buttons.defaultTitle);   /* literal "&" */ highlighted: true; enabled: buttons.defaultEnabled; onClicked: buttons.accept() }
}
