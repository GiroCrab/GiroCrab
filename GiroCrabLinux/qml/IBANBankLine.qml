import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// IBANBankLine (TransferSheetView.swift, #1563/#1584): the recipient's bank under the IBAN
// field of the transfer, standing order and mandate forms, from the Bundesbank BLZ list
// (German IBANs only). One line, laid out also without a bank: the form must not jump while
// the IBAN is typed (#881). Right-aligned like the field's text.
Text {
    id: line
    property string iban
    readonly property var bank: AppModel.bankNameForIban(iban)
    Layout.fillWidth: true
    text: bank || " "
    horizontalAlignment: Text.AlignRight
    font.pixelSize: Theme.caption
    color: Theme.secondary
    elide: Text.ElideRight
    maximumLineCount: 1
    opacity: bank ? 1 : 0
    Accessible.ignored: !bank
    HoverHandler { id: hover; enabled: !!line.bank }
    ToolTip.visible: hover.hovered && !!bank
    ToolTip.text: AppModel.masking(bank || "")
    ToolTip.delay: 600
    Sensitive { when: !!line.bank } // follows from the IBAN, often names the city
}
