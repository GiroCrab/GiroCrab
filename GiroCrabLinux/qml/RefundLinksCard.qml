import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// docs/ERSTATTUNGEN_UI.md
Card {
    id: card
    property var booking: null
    property var rows: []
    function reload() {
        rows = []
        if (!booking || !AppModel.isUnlocked) { AppModel.cancelLoad("refundCard"); return }
        const id = booking.id
        AppModel.loadOffMain("refundCard", () => [["refund_card", { transaction_id: id }]], r => {
            if (booking && booking.id === id) rows = r[0]
        }, message => { rows = []; AppModel.errorMessage = message }, true)
    }
    onBookingChanged: reload()
    Component.onDestruction: AppModel.cancelLoad("refundCard")
    Connections {
        target: AppModel
        function onRefundLinksRevisionChanged() { card.reload() }
        function onBookingsChanged() { card.reload() }
        function onVaultGenerationChanged() { card.reload() }
    }
    ColumnLayout {
        width: parent.width
        spacing: 8
        Text { text: qsTr("Erstattungen"); font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text }
        Repeater {
            model: card.rows
            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true
                ItemDelegate {
                    Layout.fillWidth: true
                    onClicked: AppModel.revealBooking(modelData.booking.id)
                    contentItem: ColumnLayout {
                        Text { text: modelData.booking.merchant_name || modelData.booking.counterparty; textFormat: Text.PlainText; font.pixelSize: Theme.body; color: Theme.text; wrapMode: Text.Wrap; Layout.fillWidth: true; Sensitive {} }
                        Text { text: Fmt.medium(modelData.booking.booking_date); font.pixelSize: Theme.caption; color: Theme.secondary }
                        Text { text: modelData.booking.account_name; textFormat: Text.PlainText; font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.Wrap; Layout.fillWidth: true; Sensitive {} }
                        Text { visible: text !== ""; text: modelData.part_label; textFormat: Text.PlainText; font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.Wrap; Layout.fillWidth: true; Sensitive {} Sensitive { labels: true } Sensitive { amounts: true } }
                        Text { text: Fmt.money(modelData.link.allocated_cents, modelData.booking.currency); font.pixelSize: Theme.body; color: Theme.text; Sensitive { amounts: true } }
                    }
                }
                ToolButton {
                    text: "×"
                    Accessible.name: qsTr("Verknüpfung aufheben")
                    enabled: !AppModel.refundEditing
                    onClicked: AppModel.changeRefundLink(modelData.link, "unlink")
                }
            }
        }
    }
}
