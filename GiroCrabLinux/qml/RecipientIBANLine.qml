import QtQuick
import QtQuick.Layouts

// RecipientIBANLine (TransferSheetView.swift, #1563/#1584): a known recipient's IBAN with its
// bank, under the name in the transfer form's suggestions and in „Empfänger verwalten“: tells
// apart several IBANs of one name. The IBAN never truncates, the bank does.
RowLayout {
    id: line
    property string iban
    property real fontSize: Theme.caption
    /// Upper bound for the whole line (chips); 0 = as wide as the layout makes it.
    property real maxWidth: 0
    readonly property var bank: AppModel.bankNameForIban(iban)
    spacing: 0
    Text {
        id: ibanText
        text: Fmt.iban(line.iban)
        font.pixelSize: line.fontSize; font.family: "monospace"; color: Theme.secondary
        Layout.alignment: Qt.AlignBaseline
        Sensitive {}
    }
    Text {
        visible: !!line.bank
        text: " · " + (line.bank || "")
        font.pixelSize: line.fontSize; color: Theme.secondary
        elide: Text.ElideRight; maximumLineCount: 1
        Layout.alignment: Qt.AlignBaseline
        Layout.fillWidth: true
        Layout.preferredWidth: line.maxWidth > 0 ? Math.min(implicitWidth, Math.max(0, line.maxWidth - ibanText.implicitWidth)) : implicitWidth
        Layout.maximumWidth: implicitWidth
        Sensitive {}
    }
}
