import QtQuick

// BatchTransferSheet.swift › TransferKindPicker (#881): „Überweisung | Umbuchung |
// Sammelüberweisung“ under the title of both transfer sheets, one switch instead of two
// stacked ones (AppModel.switchTransferKind).
Segmented {
    /// Shown in the batch sheet.
    property bool batch: false
    /// Umbuchung needs another own account with an IBAN.
    property bool offersOwn: false
    options: [{ id: "single", title: qsTr("Überweisung") }]
        .concat(offersOwn ? [{ id: "own", title: qsTr("Umbuchung") }] : [])
        .concat(Payments.canBatchTransfer ? [{ id: "batch", title: qsTr("Sammelüberweisung") }] : [])
    current: batch ? "batch" : AppModel.transferDraft.ownTransfer ? "own" : "single"
    onActivated: id => { if (id !== current) AppModel.switchTransferKind(id, batch) }
    Accessible.name: qsTr("Art")
}
