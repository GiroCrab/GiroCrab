import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// DepotActivitiesView.swift (#564): securities activities of one depot (`list_security_transactions`):
// never cash bookings of the clearing account, never used to rebuild positions. A booking on the
// clearing account is reachable through the transaction's own `cash_transaction_id` link.
ColumnLayout {
    id: av
    /// All activities of the depot; `shown` is the filtered subset.
    property var all: []
    property var shown: []
    /// DepotActivityFilter; the host owns it (`filterRequested`).
    property string filter: "all"
    signal filterRequested(string filter)
    property string query: ""
    property string periodTitle: ""
    property bool wide: true
    property var expanded: ({})
    spacing: 10

    readonly property string firstRecorded: all.map(t => t.booking_date).filter(d => d !== "").sort()[0] || ""
    readonly property real dateW: 92 * Theme.textScale
    readonly property real kindW: 124 * Theme.textScale
    readonly property real qtyW: 80 * Theme.textScale
    readonly property real valueW: 112 * Theme.textScale
    readonly property real costW: 76 * Theme.textScale
    readonly property real linkW: 28 * Theme.textScale

    /// `--expand-rows`: the first two rows start unfolded, once there are rows (screenshots).
    property bool expandedOnce: false
    onShownChanged: {
        if (expandedOnce || shown.length === 0 || AppModel.launchArgs.indexOf("--expand-rows") < 0) return
        expandedOnce = true
        const e = {}
        for (const t of shown.slice(0, 2)) e[t.id] = true
        expanded = e
    }

    function nameText(t) { return t.name !== "" ? t.name : (t.isin !== "" ? t.isin : t.wkn) }
    function idText(t) { return t.isin !== "" ? t.isin : t.wkn }
    function quantityText(t) { return t.quantity !== 0 ? NumberText.format(t.quantity) : "–" }
    function currencyOf(t) { return t.currency !== "" ? t.currency : "EUR" }
    /// Own fee and tax lines are costs, not a securities value.
    function isCostLine(t) { return t.direction === "Fee" || t.direction === "Tax" }
    function valueText(t) { return isCostLine(t) ? "–" : Fmt.money(t.value_cents, currencyOf(t)) }
    function feeText(t) { const c = DepotMetrics.feeCents(t); return c === 0 ? "–" : Fmt.money(c, currencyOf(t)) }
    function taxText(t) { const c = DepotMetrics.taxCents(t); return c === 0 ? "–" : Fmt.money(c, currencyOf(t)) }
    /// Narrow rows have one amount: the securities value, or the cost of a fee/tax line.
    function narrowAmountText(t) { return t.direction === "Fee" ? feeText(t) : t.direction === "Tax" ? taxText(t) : valueText(t) }
    function showsBankText(t) { return t.booking_text !== "" && t.booking_text !== DepotMetrics.kindTitle(t) }
    function toggle(id) { const e = Object.assign({}, expanded); if (e[id]) delete e[id]; else e[id] = true; expanded = e }

    // ContentUnavailableView „Keine Aktivitäten verfügbar“
    Unavailable {
        visible: av.all.length === 0
        symbol: "list.bullet.rectangle"; title: qsTr("Keine Aktivitäten verfügbar")
        message: qsTr("Für dieses Depot sind keine Wertpapieraktivitäten gespeichert. Nicht jede Bank liefert sie – das heißt nicht, dass es keine Geschäfte, Ausschüttungen oder Kosten gab.")
    }

    // MARK: controls
    ColumnLayout {
        visible: av.all.length > 0
        spacing: 6
        Item {
            implicitWidth: av.wide ? kindSeg.implicitWidth : kindMenuLabel.implicitWidth; implicitHeight: kindSeg.implicitHeight
            Segmented { id: kindSeg; visible: av.wide; options: DepotMetrics.activityFilters; current: av.filter; onActivated: id => av.filterRequested(id) }
            RowLayout {
                id: kindMenuLabel
                visible: !av.wide
                spacing: 4
                Text { text: (DepotMetrics.activityFilters.find(f => f.id === av.filter) || {}).title || ""; font.pixelSize: Theme.body; color: Theme.text }
                SFIcon { symbol: "chevron.down"; size: 9 }
                TapHandler { onTapped: kindMenu.popup() }
                AppMenu { id: kindMenu
                    Repeater { model: DepotMetrics.activityFilters
                        AppMenuItem { required property var modelData; text: modelData.title; checkable: true; checked: av.filter === modelData.id
                                      onTriggered: av.filterRequested(modelData.id) } } }
            }
            HoverHandler { id: kindHover }
            ToolTip.visible: kindHover.hovered; ToolTip.delay: 600
            ToolTip.text: qsTr("Kosten: eigene Gebühren- und Steuerzeilen sowie Aktivitäten mit Gebühren oder Steuern")
        }
        Text {
            text: [qsTr("%1 von %2 Aktivitäten").arg(av.shown.length).arg(av.all.length), qsTr("Zeitraum: %1").arg(av.periodTitle)]
                .concat(av.firstRecorded !== "" ? [qsTr("erfasst ab %1").arg(Fmt.medium(av.firstRecorded))] : []).join(" · ")
            font.pixelSize: Theme.caption; color: Theme.secondary
        }
    }

    // Search without hits, period without activities and an empty filter are different things.
    Unavailable {
        visible: av.all.length > 0 && av.shown.length === 0 && av.query !== ""
        symbol: "magnifyingglass"; title: qsTr("Keine Treffer")
        message: qsTr("Keine Aktivität passt zu „%1“.").arg(AppModel.redactedSearch(av.query))
    }
    Unavailable {
        visible: av.all.length > 0 && av.shown.length === 0 && av.query === ""
        symbol: "tray"; title: qsTr("Keine Aktivitäten gefunden")
        message: qsTr("Im gewählten Zeitraum und Filter sind keine gespeicherten Aktivitäten. Das ist kein Nachweis, dass keine stattgefunden haben.")
    }

    // MARK: list
    Rectangle {
        visible: av.shown.length > 0
        Layout.fillWidth: true
        implicitHeight: listCol.implicitHeight
        radius: 12; color: Theme.card; border.color: Theme.border
        clip: true
        ColumnLayout {
            id: listCol
            width: parent.width
            spacing: 0
            RowLayout {
                visible: av.wide
                Layout.fillWidth: true; Layout.leftMargin: 14; Layout.rightMargin: 14; Layout.topMargin: 8; Layout.bottomMargin: 8
                spacing: 10
                Head { title: qsTr("Datum"); Layout.preferredWidth: av.dateW; horizontalAlignment: Text.AlignLeft }
                Head { title: qsTr("Wertpapier"); Layout.fillWidth: true; Layout.preferredWidth: 1; horizontalAlignment: Text.AlignLeft }
                Head { title: qsTr("Art"); Layout.preferredWidth: av.kindW; horizontalAlignment: Text.AlignLeft }
                Head { title: qsTr("Menge"); Layout.preferredWidth: av.qtyW }
                Head { title: qsTr("Bruttobetrag"); Layout.preferredWidth: av.valueW
                       HoverHandler { id: grossHover }
                       ToolTip.visible: grossHover.hovered; ToolTip.delay: 600
                       ToolTip.text: qsTr("Bruttobetrag der Aktivität ohne Gebühren und Steuern, bei Ausschüttungen vor Quellensteuer. Gebühren und Steuern stehen in eigenen Spalten; der tatsächliche Geldbetrag steht in der verknüpften Buchung des Verrechnungskontos.") }
                Head { title: qsTr("Gebühren"); Layout.preferredWidth: av.costW }
                Head { title: qsTr("Steuern"); Layout.preferredWidth: av.costW }
                Item { Layout.preferredWidth: av.linkW }
            }
            Rectangle { visible: av.wide; Layout.fillWidth: true; implicitHeight: 1; color: Theme.border }
            Repeater {
                model: av.shown
                ColumnLayout {
                    id: row
                    required property var modelData
                    required property int index
                    readonly property var t: modelData
                    readonly property bool open: !!av.expanded[t.id]
                    Layout.fillWidth: true
                    spacing: 0
                    // wide: the columns
                    RowLayout {
                        visible: av.wide
                        Layout.fillWidth: true; Layout.leftMargin: 14; Layout.rightMargin: 14; Layout.topMargin: 5; Layout.bottomMargin: 5
                        Layout.minimumHeight: 44
                        spacing: 10
                        ColumnLayout { spacing: 1; Layout.preferredWidth: av.dateW; Layout.maximumWidth: av.dateW; Layout.minimumWidth: av.dateW
                            Text { text: Fmt.medium(row.t.booking_date); font.pixelSize: Theme.callout; color: Theme.text }
                            Text { visible: row.t.settlement_date !== "" && row.t.settlement_date !== row.t.booking_date
                                   text: qsTr("Valuta %1").arg(Fmt.medium(row.t.settlement_date)); font.pixelSize: Theme.caption2; color: Theme.tertiary } }
                        ColumnLayout { spacing: 1; Layout.fillWidth: true; Layout.preferredWidth: 1
                            Text { text: av.nameText(row.t); Layout.fillWidth: true; elide: Text.ElideRight; font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text
                                   Sensitive { labels: true } }
                            Text { visible: av.idText(row.t) !== ""; text: av.idText(row.t); Layout.fillWidth: true; elide: Text.ElideRight
                                   font.pixelSize: Theme.callout; color: Theme.secondary; Sensitive {} } }
                        ColumnLayout { spacing: 1; Layout.preferredWidth: av.kindW; Layout.maximumWidth: av.kindW; Layout.minimumWidth: av.kindW
                            Text { text: DepotMetrics.kindTitle(row.t); font.pixelSize: Theme.callout; color: Theme.text }
                            Text { visible: av.showsBankText(row.t); text: row.t.booking_text; Layout.fillWidth: true; elide: Text.ElideRight
                                   font.pixelSize: Theme.caption2; color: Theme.tertiary; Sensitive {} } }
                        Cell { text: av.quantityText(row.t); Layout.preferredWidth: av.qtyW; color: Theme.secondary }
                        Cell { text: av.valueText(row.t); Layout.preferredWidth: av.valueW; color: Theme.text }
                        Cell { text: av.feeText(row.t); Layout.preferredWidth: av.costW; color: Theme.secondary }
                        Cell { text: av.taxText(row.t); Layout.preferredWidth: av.costW; color: Theme.secondary }
                        Jump { t: row.t; Layout.preferredWidth: av.linkW }
                    }
                    // narrow: name, date and kind on the left, the amount on the right; the chevron unfolds the rest
                    RowLayout {
                        visible: !av.wide
                        Layout.fillWidth: true; Layout.leftMargin: 14; Layout.rightMargin: 14; Layout.topMargin: 5
                        Layout.bottomMargin: row.open ? 0 : 5; Layout.minimumHeight: 44
                        spacing: 10
                        ColumnLayout { spacing: 2; Layout.fillWidth: true
                            Text { text: av.nameText(row.t); Layout.fillWidth: true; elide: Text.ElideRight; wrapMode: row.open ? Text.WordWrap : Text.NoWrap
                                   maximumLineCount: row.open ? 3 : 1; font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text
                                   Sensitive { labels: true } }
                            Text { text: [Fmt.medium(row.t.booking_date), DepotMetrics.kindTitle(row.t)].join(" · "); Layout.fillWidth: true; elide: Text.ElideRight
                                   font.pixelSize: Theme.callout; color: Theme.secondary; Sensitive {} } }
                        Text { text: av.narrowAmountText(row.t); font.pixelSize: Theme.body; font.weight: Font.Medium; font.features: { "tnum": 1 }; color: Theme.text
                               Sensitive { amounts: true } }
                        // a narrow window keeps the jump at hand
                        Jump { t: row.t }
                        Item {
                            implicitWidth: 18; implicitHeight: 18
                            SFIcon { anchors.centerIn: parent; symbol: "chevron.right"; size: 10; color: Theme.secondary; rotation: row.open ? 90 : 0
                                     Behavior on rotation { NumberAnimation { duration: 200 } } }
                            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: av.toggle(row.t.id) }
                            Accessible.role: Accessible.Button
                            Accessible.name: row.open ? qsTr("Details ausblenden") : qsTr("Details anzeigen")
                        }
                    }
                    ColumnLayout {
                        visible: !av.wide && row.open
                        Layout.fillWidth: true; Layout.leftMargin: 14; Layout.rightMargin: 14; Layout.topMargin: 6; Layout.bottomMargin: 8
                        spacing: 6
                        Line { visible: row.t.settlement_date !== "" && row.t.settlement_date !== row.t.booking_date; title: qsTr("Valuta"); value: Fmt.medium(row.t.settlement_date) }
                        Line { visible: row.t.quantity !== 0; title: qsTr("Menge"); value: av.quantityText(row.t) }
                        Line { visible: !av.isCostLine(row.t); title: qsTr("Bruttobetrag"); value: av.valueText(row.t) }
                        Line { visible: av.feeText(row.t) !== "–"; title: qsTr("Gebühren"); value: av.feeText(row.t) }
                        Line { visible: av.taxText(row.t) !== "–"; title: qsTr("Steuern"); value: av.taxText(row.t) }
                        Line { visible: av.idText(row.t) !== ""; title: qsTr("ISIN / WKN"); value: av.idText(row.t) }
                        Line { visible: av.showsBankText(row.t); title: qsTr("Bankhinweis"); value: row.t.booking_text }
                    }
                    Rectangle { visible: row.index < av.shown.length - 1; Layout.fillWidth: true; Layout.leftMargin: 14; implicitHeight: 1
                                color: Theme.withAlpha(Theme.border, 0.7) }
                }
            }
        }
    }

    component Head: Text {
        property string title
        text: title; horizontalAlignment: Text.AlignRight; elide: Text.ElideRight
        font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.secondary
    }
    component Cell: Text {
        Layout.minimumWidth: Layout.preferredWidth; Layout.maximumWidth: Layout.preferredWidth
        horizontalAlignment: Text.AlignRight; elide: Text.ElideLeft
        font.pixelSize: Theme.callout; font.features: { "tnum": 1 }
        Sensitive { amounts: true }
    }
    component Line: RowLayout {
        id: line
        property string title
        property string value
        Layout.fillWidth: true
        Text { text: line.title; font.pixelSize: Theme.callout; color: Theme.secondary }
        Item { Layout.fillWidth: true; Layout.minimumWidth: 12 }
        Text { text: line.value; font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: Theme.text; horizontalAlignment: Text.AlignRight
               Layout.maximumWidth: 360 * Theme.textScale; wrapMode: Text.WordWrap; Sensitive { amounts: true } }
    }
    /// „Zur zugehörigen Buchung des Verrechnungskontos“
    component Jump: Item {
        id: jump
        property var t
        implicitWidth: 22; implicitHeight: 22
        IconButton {
            anchors.centerIn: parent
            visible: jump.t.cash_transaction_id !== null && jump.t.cash_transaction_id !== undefined
            symbol: "arrow.right.circle"; symbolSize: 15
            onClicked: AppModel.revealBooking(jump.t.cash_transaction_id)
            ToolTip.visible: hovered; ToolTip.delay: 600; ToolTip.text: qsTr("Zur zugehörigen Buchung des Verrechnungskontos")
            Accessible.name: qsTr("Zur Buchung des Verrechnungskontos springen")
        }
    }
    component Unavailable: ColumnLayout {
        id: un
        property string symbol
        property string title
        property string message
        Layout.alignment: Qt.AlignHCenter; Layout.topMargin: 20
        Layout.maximumWidth: 480
        spacing: 6
        SFIcon { Layout.alignment: Qt.AlignHCenter; symbol: un.symbol; size: 32 }
        Text { Layout.alignment: Qt.AlignHCenter; text: un.title; font.pixelSize: Theme.title2; font.weight: Font.Bold; color: Theme.text }
        Text { Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.WordWrap; text: un.message; font.pixelSize: Theme.body; color: Theme.secondary }
    }
}
