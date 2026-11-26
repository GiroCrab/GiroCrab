import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// docs/ERSTATTUNGEN_UI.md
Sheet {
    id: sheet
    visible: AppModel.refundBooking !== null
    sheetWidth: 620
    padding: 22
    property var booking: null
    property var ownParts: []
    property string ownPart: ""
    property bool fromRefund: true
    property var ownOpen: null
    property var linked: []
    property var top: []
    property var candidates: []
    property bool loading: true
    property string search: ""
    property string error: ""
    readonly property bool pending: loading || AppModel.refundEditing
    readonly property int partMasks: AppModel.redactPersonal | AppModel.redactAmounts | AppModel.redactLabels
    readonly property var others: candidates.filter(c => !linked.some(l => same(l, c)) && !top.some(s => same(s.candidate, c)))
    function same(a, b) { return a.transaction_id === b.transaction_id && a.part === b.part }
    function reasonLabel(reason) { return reason === "SameAmount" ? qsTr("Gleicher Betrag") : reason === "SameMerchant" ? qsTr("Gleicher Händler") : qsTr("Gleiche Referenz") }
    function reload() {
        if (AppModel.refundBooking === null) return
        searchTimer.stop()
        loading = true; error = ""
        linked = []; top = []; candidates = []
        const id = AppModel.refundBooking.id, part = ownPart, query = search
        AppModel.loadOffMain("refundSheet", () => [["refund_sheet", { transaction_id: id, own_part: part, query: query }]], results => {
            const data = results[0]
            booking = data.booking
            ownParts = data.parts.map((p, i) => {
                const c = AppModel.categoryByID(p.category_id)
                return { part: p.uuid, text: [qsTr("Teil %1").arg(i + 1), c ? c.path : "", p.note, Fmt.money(p.amount_cents, booking.currency)].filter(t => t !== "").join(" · ") }
            })
            ownPart = data.own_part; fromRefund = data.from_refund; ownOpen = data.own_open
            if (data.selection_invalid && ownParts.length === 0) ownParts = [{part: "", text: qsTr("Buchung")}]
            error = data.selection_invalid ? qsTr("Diese Erstattungszuordnung ist nicht möglich.") : ""
            linked = data.linked; top = data.suggestions.slice(0, 3); candidates = data.candidates
            loading = false
        }, message => { ownOpen = null; error = message; loading = false }, true)
    }
    function toggle(row) {
        if (pending || error !== "" || (!row.link && (ownOpen === 0 || row.open_cents === 0))) return
        const pair = row.link ? {
            refund_id: row.link.refund_id, refund_part: row.link.refund_part,
            original_id: row.link.original_id, original_part: row.link.original_part,
            changed_ms: row.link.changed_ms
        } : {
            refund_id: fromRefund ? booking.id : row.transaction_id,
            refund_part: fromRefund ? ownPart : row.part,
            original_id: fromRefund ? row.transaction_id : booking.id,
            original_part: fromRefund ? row.part : ownPart
        }
        AppModel.changeRefundLink(pair, row.link ? "unlink" : "link")
    }
    onAboutToShow: reload()
    onClosed: { searchTimer.stop(); AppModel.cancelLoad("refundSheet"); AppModel.refundBooking = null }
    property Timer searchTimer: Timer { interval: 200; onTriggered: sheet.reload() }
    onSearchChanged: {
        AppModel.cancelLoad("refundSheet")
        loading = true; linked = []; top = []; candidates = []
        searchTimer.restart()
    }
    property Connections changes: Connections {
        target: AppModel
        function onRefundLinksRevisionChanged() { sheet.reload() }
        function onBookingsChanged() { sheet.reload() }
        function onCategoriesChanged() { sheet.reload() }
        function onVaultGenerationChanged() { sheet.close() }
    }

    component RefundRow: ItemDelegate {
        id: rr
        required property var row
        property string reason: ""
        Layout.fillWidth: true
        enabled: !sheet.pending && sheet.error === "" && (!!row.link || (sheet.ownOpen > 0 && row.open_cents > 0))
        opacity: enabled ? 1 : 0.45
        padding: 7
        onClicked: sheet.toggle(row)
        Accessible.name: AppModel.masking(row.merchant_name, AppModel.redactPersonal)
        ToolTip.visible: hovered; ToolTip.delay: 600
        ToolTip.text: row.link ? qsTr("Verknüpfung aufheben") : row.open_cents === 0 ? qsTr("Diese Buchung ist bereits vollständig erstattet.") : qsTr("Mit dieser Buchung verknüpfen")
        contentItem: RowLayout {
            spacing: 10
            SFIcon { symbol: rr.row.link ? "checkmark.circle.fill" : "circle"; size: 18; color: rr.row.link ? Theme.accent : Theme.tertiary }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Text { text: rr.row.merchant_name; textFormat: Text.PlainText; font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text; elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} }
                Text { text: [Fmt.medium(rr.row.booking_date), rr.row.account_name, rr.row.part_label].filter(t => t !== "").join(" · "); textFormat: Text.PlainText; font.pixelSize: Theme.caption; color: Theme.secondary; elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} Sensitive { labels: true; when: rr.row.part_label !== "" } Sensitive { amounts: true; when: rr.row.part_label !== "" } }
                Text { text: rr.row.purpose; textFormat: Text.PlainText; visible: text !== ""; font.pixelSize: Theme.caption; color: Theme.secondary; elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} }
                Text { visible: rr.reason !== ""; text: sheet.reasonLabel(rr.reason); font.pixelSize: Theme.caption; color: Theme.accent }
            }
            ColumnLayout {
                Text { Layout.alignment: Qt.AlignRight; text: Fmt.money(rr.row.amount_cents, rr.row.currency); font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text; Sensitive { amounts: true } }
                Text { Layout.alignment: Qt.AlignRight; visible: !!rr.row.link; text: qsTr("Zugeordnet: %1").arg(Fmt.money(rr.row.allocated_cents || 0, rr.row.currency)); font.pixelSize: Theme.caption; color: Theme.secondary; Sensitive { amounts: true } }
            }
        }
    }

    ColumnLayout {
        width: sheet.availableWidth
        spacing: 10
        RowLayout {
            Layout.fillWidth: true
            Text { Layout.fillWidth: true; text: sheet.fromRefund ? qsTr("Ursprüngliche Buchung zuordnen") : qsTr("Erstattung zuordnen"); font.pixelSize: Theme.title3; font.weight: Font.Bold; color: Theme.text }
            Button { text: qsTr("Fertig"); highlighted: true; onClicked: sheet.close() }
        }
        Text {
            Layout.fillWidth: true
            text: sheet.booking ? sheet.booking.merchant_name + " · " + Fmt.money(sheet.booking.amount_cents, sheet.booking.currency) : ""
            textFormat: Text.PlainText; elide: Text.ElideRight; font.pixelSize: Theme.body; color: Theme.secondary
            Sensitive {} Sensitive { amounts: true }
        }
        Text { visible: sheet.ownParts.length > 0; text: sheet.fromRefund ? qsTr("Erstattung aus") : qsTr("Erstattung für"); font.pixelSize: Theme.caption; color: Theme.secondary }
        AppComboBox {
            id: partPicker
            visible: sheet.ownParts.length > 0
            enabled: !sheet.pending
            Layout.fillWidth: true
            model: sheet.ownParts.map(p => p.text)
            currentIndex: sheet.ownParts.findIndex(p => p.part === sheet.ownPart)
            displayText: AppModel.masking(sheet.ownParts[currentIndex] ? sheet.ownParts[currentIndex].text : "", sheet.partMasks)
            contentItem: Text { text: partPicker.displayText; textFormat: Text.PlainText; font.pixelSize: Theme.body; color: Theme.text; elide: Text.ElideRight; verticalAlignment: Text.AlignVCenter }
            onActivated: index => { sheet.ownPart = sheet.ownParts[index].part; sheet.reload() }
            Accessible.name: sheet.fromRefund ? qsTr("Erstattung aus") : qsTr("Erstattung für")
            Sensitive {} Sensitive { labels: true } Sensitive { amounts: true }
            delegate: AppMenuItem {
                required property var modelData
                required property int index
                width: partPicker.width
                text: AppModel.masking(modelData, sheet.partMasks)
                contentItem: Text { text: AppModel.masking(modelData, sheet.partMasks); textFormat: Text.PlainText; font.pixelSize: Theme.body; color: Theme.text; elide: Text.ElideRight; verticalAlignment: Text.AlignVCenter }
                highlighted: partPicker.highlightedIndex === index
                Sensitive {} Sensitive { labels: true } Sensitive { amounts: true }
            }
        }
        TextField { id: searchField; Layout.fillWidth: true; placeholderText: qsTr("Name oder Verwendungszweck suchen"); onTextEdited: sheet.search = text; Sensitive { when: searchField.text !== "" } }
        Text { Layout.fillWidth: true; visible: sheet.error !== ""; text: sheet.error; textFormat: Text.PlainText; font.pixelSize: Theme.callout; color: Theme.negative; wrapMode: Text.WordWrap }
        Text { visible: sheet.ownOpen !== null; text: qsTr("Noch offen: %1").arg(sheet.booking ? Fmt.money(sheet.ownOpen || 0, sheet.booking.currency) : ""); font.pixelSize: Theme.caption; color: Theme.secondary; Sensitive { amounts: true } }
        BusyIndicator { running: sheet.pending; visible: running; Layout.alignment: Qt.AlignHCenter; Layout.preferredHeight: 24 }
        ScrollView {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(100, Math.min(400, sheet.maxBodyHeight - 240))
            clip: true
            contentWidth: availableWidth
            ColumnLayout {
                width: parent.width
                spacing: 2
                Text { visible: sheet.linked.length > 0; text: sheet.fromRefund ? qsTr("Verknüpfte ursprüngliche Buchungen (%1)").arg(sheet.linked.length) : qsTr("Verknüpfte Erstattungen (%1)").arg(sheet.linked.length); font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.secondary }
                Repeater { model: sheet.linked; RefundRow { required property var modelData; row: modelData } }
                Text { visible: sheet.top.length > 0; text: qsTr("Vorschläge"); font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.secondary; Layout.topMargin: 8 }
                Repeater { model: sheet.top; RefundRow { required property var modelData; row: modelData.candidate; reason: modelData.reason } }
                Text { visible: sheet.others.length > 0; text: sheet.search.trim() !== "" ? qsTr("Suchergebnisse") : qsTr("Weitere Buchungen"); font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.secondary; Layout.topMargin: 8 }
                Repeater { model: sheet.others; RefundRow { required property var modelData; row: modelData } }
                Text { visible: !sheet.pending && sheet.error === "" && (sheet.search.trim() !== "" ? sheet.candidates.length === 0 : sheet.linked.length === 0 && sheet.top.length === 0 && sheet.others.length === 0); text: qsTr("Keine passenden Buchungen"); font.pixelSize: Theme.callout; color: Theme.secondary }
            }
        }
    }
}
