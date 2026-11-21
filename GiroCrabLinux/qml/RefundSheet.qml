import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// RefundsUI.swift › RefundPickerSheet (#714, docs/ERSTATTUNGEN_UI.md): link a refund to the
// bookings it pays back, or a booking to its refunds. Linked rows on top with their share,
// up to three suggestions, then the searchable list. One click links or removes one pair.
Sheet {
    id: sheet
    readonly property var booking: AppModel.refundBooking
    readonly property bool fromRefund: booking !== null && booking.amount_cents > 0
    property var links: null
    property var top: []
    property var candidates: []
    property string search: ""
    property string error: ""
    visible: booking !== null
    sheetWidth: 580
    padding: 22
    onClosed: AppModel.refundBooking = null
    onAboutToShow: reload()

    readonly property var linked: links ? (fromRefund ? links.originals : links.refunds) : []
    /// The booking has nothing left for a further link.
    readonly property bool usedUp: links !== null && links.open_cents === 0
    /// Candidates below the suggestions; a linked or suggested booking is shown once only.
    readonly property var others: candidates.filter(c => !linked.some(l => l.id === c.id) && !top.some(s => s.booking.id === c.id))

    property Connections changes: Connections {
        target: AppModel
        function onRefundLinksRevisionChanged() { sheet.reload() }
    }
    property Timer searchTimer: Timer { interval: 200; onTriggered: sheet.reload() }
    onSearchChanged: searchTimer.restart()

    function reload() {
        const b = booking
        if (b === null) return
        try {
            links = AppModel.core("refund_links", { transaction_id: b.id })
            top = sheet.search.trim() === "" ? AppModel.core("refund_suggestions", { transaction_id: b.id }) : []
            candidates = AppModel.core("refund_candidates", { transaction_id: b.id, query: sheet.search, limit: 200 })
            error = ""
        } catch (e) { error = e.message }
    }
    function toggle(row, isLinked) {
        AppModel.setRefundLink(fromRefund ? booking.id : row.id, fromRefund ? row.id : booking.id, !isLinked)
    }
    /// Why an unlinked row cannot be chosen, "" when it can.
    function blocked(row) {
        if (usedUp) return fromRefund ? qsTr("Die Erstattung ist bereits vollständig zugeordnet.") : qsTr("Die Buchung ist bereits vollständig erstattet.")
        if (row.open_cents === 0) return fromRefund ? qsTr("Diese Buchung ist bereits vollständig erstattet.") : qsTr("Diese Erstattung ist bereits vollständig zugeordnet.")
        return ""
    }
    function reasonLabel(reason) {
        return reason === "SameAmount" ? qsTr("Gleicher Betrag") : reason === "SameMerchant" ? qsTr("Gleicher Händler") : qsTr("Gleiche Referenz")
    }
    /// RefundSum.lines: the sums of the booking, never above its amount.
    readonly property var sumLines: {
        if (links === null || booking === null) return []
        const money = c => AppModel.masking(Fmt.money(c, booking.currency), AppModel.redactAmounts)
        const total = Math.abs(booking.amount_cents)
        const out = []
        if (!fromRefund && links.refunded_cents !== null) out.push(qsTr("Erstattet: %1 von %2").arg(money(links.refunded_cents)).arg(money(total)))
        if (fromRefund && links.assigned_cents !== null) {
            out.push(qsTr("Zugeordnet: %1 von %2").arg(money(links.assigned_cents)).arg(money(total)))
            if (links.assigned_cents < total) out.push(qsTr("Noch offen: %1. Sie können weitere Buchungen zuordnen.").arg(money(total - links.assigned_cents)))
        }
        return out
    }

    component RefundRow: ItemDelegate {
        id: rr
        required property var row
        property bool isLinked: false
        property string reason: ""
        readonly property string why: isLinked ? "" : sheet.blocked(row)
        Layout.fillWidth: true
        enabled: why === ""
        opacity: enabled ? 1 : 0.45
        padding: 6
        onClicked: sheet.toggle(row, isLinked)
        ToolTip.visible: hovered; ToolTip.delay: 600
        ToolTip.text: why !== "" ? why : isLinked ? qsTr("Verknüpfung aufheben") : qsTr("Mit dieser Buchung verknüpfen")
        Accessible.name: row.display_name
        contentItem: RowLayout {
            spacing: 10
            // the circle is part of the row: one click selects
            SFIcon { symbol: rr.isLinked ? "checkmark.circle.fill" : "circle"; size: 18; color: rr.isLinked ? Theme.accent : Theme.tertiary }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 1
                Text { text: rr.row.display_name; font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text; elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} }
                Text { text: Fmt.date(rr.row.booking_date) + " · " + rr.row.account_name; font.pixelSize: Theme.caption; color: Theme.secondary; elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} }
                Text { visible: text !== ""; text: rr.row.purpose; font.pixelSize: Theme.caption; color: Theme.secondary; elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} }
            }
            Text {
                visible: rr.reason !== ""
                text: sheet.reasonLabel(rr.reason)
                font.pixelSize: Theme.caption; font.weight: Font.Medium; color: Theme.accent
            }
            ColumnLayout {
                spacing: 1
                Text { Layout.alignment: Qt.AlignRight; text: Fmt.money(rr.row.amount_cents, rr.row.currency); font.pixelSize: Theme.body; font.weight: Font.DemiBold; font.features: { "tnum": 1 }; color: Theme.text; Sensitive { amounts: true } }
                // what this link takes of the refund: at most the booking's amount, at most the rest
                Text {
                    Layout.alignment: Qt.AlignRight
                    visible: rr.isLinked && rr.row.share_cents !== null && rr.row.share_cents !== undefined
                    text: visible ? qsTr("Zugeordnet: %1").arg(Fmt.money(rr.row.share_cents, rr.row.currency)) : ""
                    font.pixelSize: Theme.caption; font.features: { "tnum": 1 }
                    color: rr.row.share_cents === 0 ? Theme.warning : Theme.secondary
                    Sensitive { amounts: true }
                }
            }
        }
    }

    ColumnLayout {
        width: sheet.availableWidth
        spacing: 10
        RowLayout {
            Layout.fillWidth: true
            Text { text: sheet.fromRefund ? qsTr("Ursprüngliche Buchung zuordnen") : qsTr("Erstattung zuordnen"); font.pixelSize: Theme.title3 + 1; font.weight: Font.Bold; color: Theme.text; Layout.fillWidth: true }
            Button { text: qsTr("Fertig"); highlighted: true; onClicked: sheet.close() }
        }
        Text {
            text: sheet.booking ? sheet.booking.merchant_name + " · " + Fmt.money(sheet.booking.amount_cents, sheet.booking.currency) : ""
            font.pixelSize: Theme.body; color: Theme.secondary
            Sensitive { amounts: true }
        }
        TextField {
            Layout.fillWidth: true
            placeholderText: qsTr("Name oder Verwendungszweck suchen")
            onTextEdited: sheet.search = text
        }
        Text { visible: sheet.error !== ""; text: sheet.error; font.pixelSize: Theme.callout + 1; color: Theme.negative; wrapMode: Text.WordWrap; Layout.fillWidth: true }
        ScrollView {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(440, sheet.maxBodyHeight - 150)
            clip: true
            contentWidth: availableWidth
            ColumnLayout {
                width: parent.width
                spacing: 2
                Text {
                    visible: sheet.linked.length > 0
                    text: sheet.fromRefund ? qsTr("Verknüpfte ursprüngliche Buchungen (%1)").arg(sheet.linked.length) : qsTr("Verknüpfte Erstattungen (%1)").arg(sheet.linked.length)
                    font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.secondary
                }
                Repeater {
                    model: sheet.sumLines
                    Text { required property string modelData; text: modelData; font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap; Layout.fillWidth: true }
                }
                // in the order the links were made: the first one is filled first
                Repeater { model: sheet.linked; RefundRow { required property var modelData; row: modelData; isLinked: true } }
                Text {
                    visible: sheet.usedUp && (sheet.top.length > 0 || sheet.others.length > 0)
                    text: sheet.fromRefund
                        ? qsTr("Die Erstattung ist vollständig zugeordnet. Heben Sie eine Verknüpfung auf, um eine andere Buchung zu wählen.")
                        : qsTr("Die Buchung ist vollständig erstattet. Heben Sie eine Verknüpfung auf, um eine andere Erstattung zu wählen.")
                    font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap; Layout.fillWidth: true; Layout.topMargin: 4
                }
                Text { visible: sheet.top.length > 0; text: qsTr("Vorschläge"); font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.secondary; Layout.topMargin: 8 }
                Repeater { model: sheet.top; RefundRow { required property var modelData; row: modelData.booking; reason: modelData.reason } }
                Text {
                    visible: sheet.others.length > 0
                    text: sheet.search.trim() !== "" ? qsTr("Suchergebnisse") : qsTr("Weitere Buchungen")
                    font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.secondary; Layout.topMargin: 8
                }
                Repeater { model: sheet.others; RefundRow { required property var modelData; row: modelData } }
                Text {
                    visible: sheet.linked.length === 0 && sheet.top.length === 0 && sheet.others.length === 0 && sheet.error === ""
                    text: qsTr("Keine passenden Buchungen")
                    font.pixelSize: Theme.callout; color: Theme.secondary; Layout.alignment: Qt.AlignHCenter; Layout.topMargin: 30
                }
            }
        }
    }
}
