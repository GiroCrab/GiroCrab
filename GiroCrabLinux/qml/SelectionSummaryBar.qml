import QtQuick
import QtQuick.Layouts

// BookingListView.swift › SelectionSummaryBar (#878): bar below the list while several bookings are
// selected – count and sum over all accounts, one sum per currency (AppModel+Selection.swift ›
// selectionTotals: currencies are never added together, sorted by currency code).
Rectangle {
    id: bar
    readonly property bool shown: AppModel.isMultiSelection
    /// [{ currency, cents }] of the selected bookings.
    readonly property var totals: {
        if (!shown) return []
        const sums = {}
        for (const b of AppModel.selectedBookings) {
            const account = AppModel.accountsByID[b.account_id]
            const c = b.currency || (account && account.currency) || "EUR"
            sums[c] = (sums[c] || 0) + b.amount_cents
        }
        return Object.keys(sums).sort().map(c => ({ currency: c, cents: sums[c] }))
    }
    visible: shown
    implicitHeight: shown ? content.implicitHeight + 20 : 0
    // .background(.bar) with a divider on top
    color: Theme.sidebar
    Rectangle { anchors.top: parent.top; width: parent.width; height: 1; color: Theme.border }

    RowLayout {
        id: content
        anchors.fill: parent; anchors.leftMargin: 20; anchors.rightMargin: 20; anchors.topMargin: 10; anchors.bottomMargin: 10
        spacing: 12
        Text {
            Layout.alignment: Qt.AlignTop
            text: qsTr("%1 Buchungen ausgewählt").arg(AppModel.selectedBookings.length)
            font.pixelSize: Theme.callout; font.weight: Font.Medium; font.features: { "tnum": 1 }; color: Theme.text
        }
        Item { Layout.fillWidth: true; Layout.minimumWidth: 12 }
        ColumnLayout {
            spacing: 2
            Repeater {
                model: bar.totals
                // InlineSumStyle: "Summe  1.234,56 €" on one line
                RowLayout {
                    required property var modelData
                    Layout.alignment: Qt.AlignRight
                    spacing: 8
                    Text { text: qsTr("Summe"); font.pixelSize: Theme.callout; color: Theme.secondary }
                    Text {
                        text: Fmt.money(modelData.cents, modelData.currency)
                        font.pixelSize: Theme.callout; font.weight: Font.DemiBold; font.features: { "tnum": 1 }; color: Theme.outflow
                        Sensitive { amounts: true }
                    }
                }
            }
        }
    }
}
