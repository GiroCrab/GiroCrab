import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtCore

// AssetCompositionCard.swift › SliceCompositionCard and CompositionCharts.swift (#564):
// "Zusammensetzung" from finished slices: the grouping control of the caller, the drawing (ring,
// ranked bars or treemap) with one shared colour per slice, the click filter and the notes below.
// The drawings compute no sums of their own, so switching the display never changes a number.
// Amounts, shares and the drawings are sensitive; names are labels (own items: personal).
AssetSectionCard {
    id: card
    /// DepotSlice list, largest first; folding for the drawing happens here.
    property var slices: []
    property string currency: "EUR"
    /// Whether the position-like slices fold into "Weitere (n)" in the ring (not for classes).
    property bool foldsRing: true
    /// The memberKeys of the clicked slice (null: all). The host owns it: `filterRequested`.
    property var filter: null
    signal filterRequested(var keys)
    property string displayKey
    property string note: ""
    property string emptyText: qsTr("Keine bewerteten Positionen")
    property alias grouping: groupingSlot.data
    property alias footer: footerSlot.data

    title: qsTr("Zusammensetzung")

    /// @AppStorage(displayKey): ring | bars | tiles
    property string display: "ring"
    Settings { id: displayStore; category: "holdings" }
    function loadDisplay() { const v = "" + displayStore.value(displayKey, "ring"); display = ["ring", "bars", "tiles"].indexOf(v) >= 0 ? v : "ring" }
    Component.onCompleted: loadDisplay()
    onDisplayKeyChanged: loadDisplay()
    function setDisplay(d) { if (d === display) return; display = d; displayStore.setValue(displayKey, d); filterRequested(null) }

    readonly property real total: slices.reduce((t, s) => t + s.cents, 0)
    readonly property var ringSlices: foldsRing ? DepotMetrics.foldSlices(slices, DepotMetrics.maxPositionSlices) : slices
    readonly property var tileSlices: DepotMetrics.foldSlices(slices, DepotMetrics.maxTiles)
    /// The clicked slice, as long as it still exists in the drawing that shows it.
    readonly property string selected: {
        if (!filter) return ""
        const drawn = display === "tiles" ? tileSlices : display === "ring" ? ringSlices : slices
        const hit = drawn.find(s => DepotMetrics.sameKeys(s.memberKeys, filter))
        return hit ? hit.id : ""
    }
    /// A click on a slice filters the list to its positions; the same slice again ends it.
    function choose(s) { filterRequested(DepotMetrics.sameKeys(filter, s.memberKeys) ? null : s.memberKeys) }
    function shareText(cents) { return total > 0 ? DepotMetrics.percent(cents / total * 100, false, 1) : "–" }
    /// The name as read aloud: masked like the drawn one (personal for an own item, else a label).
    function spokenLabel(s) { return AppModel.masking(s.label, s.personal ? AppModel.redactPersonal : AppModel.redactLabels) }
    /// „Name · Wert · Anteil · n Positionen“ (tooltip and spoken name): name, amount and share masked like their drawn texts.
    function spokenDetail(s) {
        const positions = s.count > 1 ? " · " + qsTr("%1 Positionen").arg(s.count) : ""
        return spokenLabel(s) + " · " + AppModel.masking(Fmt.money(s.cents, currency), AppModel.redactAmounts)
            + " · " + AppModel.masking(shareText(s.cents), AppModel.redactAmounts) + positions
    }

    // grouping and drawing, side by side or stacked
    Flow {
        Layout.fillWidth: true
        spacing: 12
        Row { id: groupingSlot; spacing: 8 }
        // icons like the chart switch of the account header (#634)
        Rectangle {
            implicitWidth: displayRow.implicitWidth + 4; implicitHeight: 22
            width: implicitWidth; height: implicitHeight
            radius: 6
            color: Theme.withAlpha(Theme.secondary, 0.12)
            Row {
                id: displayRow
                x: 2; y: 2
                Repeater {
                    model: [{ id: "ring", symbol: "chart.pie", title: qsTr("Kreis") }, { id: "bars", symbol: "chart.bar.xaxis", title: qsTr("Balken") },
                            { id: "tiles", symbol: "square.grid.2x2", title: qsTr("Kacheln") }]
                    Rectangle {
                        id: segment
                        required property var modelData
                        width: 30; height: 18; radius: 5
                        color: card.display === modelData.id ? Theme.card : "transparent"
                        border.color: card.display === modelData.id ? Theme.border : "transparent"
                        SFIcon { anchors.centerIn: parent; symbol: modelData.symbol; size: 12; color: Theme.text }
                        MouseArea { id: segArea; anchors.fill: parent; hoverEnabled: true; onClicked: card.setDisplay(modelData.id) }
                        ToolTip.visible: segArea.containsMouse; ToolTip.delay: 600; ToolTip.text: modelData.title
                        // a segmented Picker on the Mac: Tab reaches each segment, Space / Return picks it
                        activeFocusOnTab: true
                        Keys.onSpacePressed: card.setDisplay(modelData.id)
                        Keys.onReturnPressed: card.setDisplay(modelData.id)
                        Keys.onEnterPressed: card.setDisplay(modelData.id)
                        Rectangle { visible: segment.activeFocus; anchors.fill: parent; anchors.margins: -2
                                    radius: 6; color: "transparent"; border.width: 2; border.color: Theme.accent }
                        Accessible.role: Accessible.RadioButton; Accessible.name: modelData.title; Accessible.checked: card.display === modelData.id
                        Accessible.onPressAction: card.setDisplay(modelData.id)
                    }
                }
            }
            HoverHandler { id: displayHover }
        }
    }
    Text { visible: card.note !== ""; text: card.note; font.pixelSize: Theme.caption; color: Theme.secondary; Layout.fillWidth: true; wrapMode: Text.WordWrap }
    Text { visible: card.total <= 0; text: card.emptyText; font.pixelSize: Theme.caption; color: Theme.secondary }

    // MARK: ring – segments proportional to the market value, the total in the middle, the legend beside or below
    GridLayout {
        visible: card.total > 0 && card.display === "ring"
        Layout.fillWidth: true
        readonly property real ringSize: 168 * Theme.textScale
        columns: card.width >= ringSize + 28 + 320 * Theme.textScale ? 3 : 1
        columnSpacing: 28; rowSpacing: 14
        Item {
            id: ring
            Layout.alignment: Qt.AlignHCenter | Qt.AlignVCenter
            implicitWidth: parent.ringSize; implicitHeight: parent.ringSize
            readonly property var drawn: card.ringSlices.filter(s => s.cents > 0)
            onDrawnChanged: canvas.requestPaint()
            Connections { target: card; function onSelectedChanged() { canvas.requestPaint() } }
            Connections { target: Theme; function onDarkChanged() { canvas.requestPaint() } }
            Canvas {
                id: canvas
                anchors.fill: parent
                onPaint: {
                    const ctx = getContext("2d")
                    ctx.reset()
                    const sum = ring.drawn.reduce((t, s) => t + s.cents, 0)
                    if (sum <= 0) return
                    const cx = width / 2, cy = height / 2, r = Math.min(width, height) / 2, inner = r * 0.66
                    const inset = ring.drawn.length > 1 ? 1.5 : 0
                    let a = -Math.PI / 2
                    for (const s of ring.drawn) {
                        const span = s.cents / sum * 2 * Math.PI
                        const o = inset / r, i = inset / inner
                        if (span > 2 * o) {
                            ctx.beginPath()
                            ctx.arc(cx, cy, r, a + o, a + span - o, false)
                            ctx.arc(cx, cy, inner, a + span - Math.min(i, span / 2), a + Math.min(i, span / 2), true)
                            ctx.closePath()
                            ctx.globalAlpha = card.selected === "" || card.selected === s.id ? 1 : 0.3
                            ctx.fillStyle = s.color
                            ctx.fill()
                        }
                        a += span
                    }
                    ctx.globalAlpha = 1
                }
            }
            Sensitive {}
            TapHandler {
                onTapped: ev => {
                    const dx = ev.position.x - ring.width / 2, dy = ev.position.y - ring.height / 2
                    const distance = Math.hypot(dx, dy), radius = Math.min(ring.width, ring.height) / 2
                    if (distance < radius * 0.66 || distance > radius) return
                    let angle = Math.atan2(dx, -dy)
                    if (angle < 0) angle += 2 * Math.PI
                    const sum = ring.drawn.reduce((t, s) => t + s.cents, 0)
                    let end = 0
                    for (const s of ring.drawn) {
                        end += s.cents / sum * 2 * Math.PI
                        if (angle < end) { card.choose(s); return }
                    }
                }
            }
            ColumnLayout {
                anchors.centerIn: parent
                spacing: 0
                Text { Layout.alignment: Qt.AlignHCenter; text: Fmt.money(card.total, card.currency)
                       font.pixelSize: Theme.callout + 1; font.weight: Font.DemiBold; font.features: { "tnum": 1 }; color: Theme.text
                       Sensitive { amounts: true } }
                Text { Layout.alignment: Qt.AlignHCenter; text: qsTr("Summe"); font.pixelSize: Theme.caption2; color: Theme.secondary }
            }
            Accessible.role: Accessible.Chart
            Accessible.name: qsTr("Kreisdiagramm der Zusammensetzung, %1 Segmente").arg(card.ringSlices.length)
        }
        Legend {
            Layout.fillWidth: true; Layout.maximumWidth: 560 * Theme.textScale
            Layout.alignment: Qt.AlignVCenter
            slices: card.ringSlices
        }
        Item { visible: parent.columns === 3; Layout.fillWidth: true }
    }

    // MARK: ranked bars – one bar per position or group on a common scale from zero, the rest on its own
    ColumnLayout {
        id: bars
        visible: card.total > 0 && card.display === "bars"
        Layout.fillWidth: true
        spacing: 8
        readonly property var ranking: DepotMetrics.ranked(card.slices, DepotMetrics.maxRankedSlices)
        readonly property real scale: Math.max(ranking.shown.reduce((m, s) => Math.max(m, s.cents), 1), 1)
        Repeater {
            model: bars.ranking.shown
            RowLayout {
                id: barRow
                required property var modelData
                Layout.fillWidth: true
                spacing: 10
                opacity: card.selected === "" || card.selected === modelData.id ? 1 : 0.35
                Text { text: barRow.modelData.label; textFormat: Text.PlainText; Layout.preferredWidth: 150 * Theme.textScale; wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight
                       font.pixelSize: Theme.caption; color: barRow.modelData.unclassified ? Theme.secondary : Theme.text
                       Sensitive { labels: !barRow.modelData.personal }
                       FocusFrame { target: barRow } }
                Item {
                    Layout.fillWidth: true; implicitHeight: 12 * Theme.textScale
                    Rectangle { height: parent.height; radius: 3; color: barRow.modelData.color
                                width: Math.max(parent.width * barRow.modelData.cents / bars.scale, 2) }
                    Sensitive {}
                }
                Text { text: Fmt.money(barRow.modelData.cents, card.currency); Layout.minimumWidth: 96 * Theme.textScale; horizontalAlignment: Text.AlignRight
                       font.pixelSize: Theme.caption; font.features: { "tnum": 1 }; color: Theme.secondary; Sensitive { amounts: true } }
                Text { text: card.shareText(barRow.modelData.cents); Layout.minimumWidth: 46 * Theme.textScale; horizontalAlignment: Text.AlignRight
                       font.pixelSize: Theme.caption; font.features: { "tnum": 1 }; color: Theme.secondary; Sensitive { amounts: true } }
                TapHandler { onTapped: card.choose(barRow.modelData) }
                HoverHandler { id: barArea; cursorShape: Qt.PointingHandCursor }
                ToolTip.visible: barArea.hovered; ToolTip.delay: 600; ToolTip.text: card.spokenDetail(barRow.modelData)
                // a Button on the Mac: Tab reaches it, Space / Return filters
                activeFocusOnTab: true
                Keys.onSpacePressed: card.choose(barRow.modelData)
                Keys.onReturnPressed: card.choose(barRow.modelData)
                Keys.onEnterPressed: card.choose(barRow.modelData)
                Accessible.role: Accessible.Button
                Accessible.name: card.spokenDetail(barRow.modelData)
                Accessible.checked: card.selected === barRow.modelData.id
                Accessible.onPressAction: card.choose(barRow.modelData)
            }
        }
        Rectangle { visible: bars.ranking.rest !== null; Layout.fillWidth: true; implicitHeight: 1; color: Theme.border }
        RowLayout {
            visible: bars.ranking.rest !== null
            Layout.fillWidth: true
            spacing: 10
            Text { text: bars.ranking.rest ? bars.ranking.rest.label : ""; Layout.preferredWidth: 150 * Theme.textScale; font.pixelSize: Theme.caption; color: Theme.secondary }
            Item { Layout.fillWidth: true }
            Text { text: bars.ranking.rest ? Fmt.money(bars.ranking.rest.cents, card.currency) : ""; Layout.minimumWidth: 96 * Theme.textScale; horizontalAlignment: Text.AlignRight
                   font.pixelSize: Theme.caption; font.features: { "tnum": 1 }; color: Theme.secondary; Sensitive { amounts: true } }
            Text { text: bars.ranking.rest ? card.shareText(bars.ranking.rest.cents) : ""; Layout.minimumWidth: 46 * Theme.textScale; horizontalAlignment: Text.AlignRight
                   font.pixelSize: Theme.caption; font.features: { "tnum": 1 }; color: Theme.secondary; Sensitive { amounts: true } }
            HoverHandler { id: restHover }
            ToolTip.visible: restHover.hovered; ToolTip.delay: 600; ToolTip.text: qsTr("Die kleineren Positionen stehen vollständig in der Positionsliste.")
        }
    }

    // MARK: treemap – area ∝ market value, colour of the slice, name and „Wert · Anteil“ where they fit
    Item {
        id: treemap
        visible: card.total > 0 && card.display === "tiles"
        Layout.fillWidth: true
        implicitHeight: Math.min(210 * Theme.textScale, Math.max(150 * Theme.textScale, width * 0.6))
        property string hovered: ""
        /// the tile with keyboard focus, apart from the pointer: leaving another tile must not hide its detail card
        property string focused: ""
        readonly property var rects: DepotMetrics.treemap(card.tileSlices.map(s => s.cents), width, height, card.tileSlices.length > 4 ? 2.4 : 0)
        readonly property real charWidth: 6.6 * Theme.textScale
        readonly property var focusSlice: { const id = hovered !== "" ? hovered : focused !== "" ? focused : card.selected; return card.tileSlices.find(s => s.id === id) || null }
        Repeater {
            model: card.tileSlices.length
            Rectangle {
                id: tile
                required property int index
                readonly property var s: card.tileSlices[index]
                readonly property var r: treemap.rects[index] || { x: 0, y: 0, w: 0, h: 0 }
                readonly property color ink: DepotMetrics.tileInk(s.color)
                readonly property string share: card.shareText(s.cents)
                readonly property string amountLine: Fmt.money(s.cents, card.currency) + " · " + share
                readonly property real inner: r.w - 16
                readonly property bool fullFits: inner >= amountLine.length * treemap.charWidth
                readonly property bool shareFits: inner >= share.length * treemap.charWidth
                readonly property bool nameFits: r.w > 56 * Theme.textScale && r.h > 26 * Theme.textScale
                readonly property bool both: fullFits && nameFits && r.h > 44 * Theme.textScale
                readonly property bool nameOnly: !both && nameFits
                readonly property bool shareOnly: !both && !nameOnly && shareFits && r.h > 18 * Theme.textScale
                x: r.x + 1.5; y: r.y + 1.5
                width: Math.max(r.w - 3, 0); height: Math.max(r.h - 3, 0)
                radius: 6
                color: s.color
                opacity: card.selected === "" || card.selected === s.id ? 1 : 0.35
                clip: true
                Column {
                    x: 8; y: 8
                    width: parent.width - 16
                    spacing: 1
                    Text { visible: tile.both || tile.nameOnly; text: tile.s.label; textFormat: Text.PlainText; width: parent.width; elide: Text.ElideRight
                           font.pixelSize: Theme.caption; font.weight: Font.Bold; color: tile.ink
                           Sensitive { labels: !tile.s.personal } }
                    Text { visible: tile.both; text: tile.amountLine; width: parent.width; elide: Text.ElideRight
                           font.pixelSize: Theme.caption2; font.features: { "tnum": 1 }; color: tile.ink; Sensitive { amounts: true } }
                    Text { visible: (tile.nameOnly && tile.shareFits && tile.r.h > 44 * Theme.textScale) || tile.shareOnly; text: tile.share
                           font.pixelSize: Theme.caption2; font.weight: tile.shareOnly ? Font.DemiBold : Font.Normal; font.features: { "tnum": 1 }; color: tile.ink
                           Sensitive { amounts: true } }
                }
                MouseArea {
                    anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
                    onContainsMouseChanged: { if (containsMouse) treemap.hovered = tile.s.id; else if (treemap.hovered === tile.s.id) treemap.hovered = "" }
                    onClicked: card.choose(tile.s)
                }
                // Tab reaches the tile, Space / Return filters; the detail card follows the focus like the pointer
                activeFocusOnTab: true
                Keys.onSpacePressed: card.choose(tile.s)
                Keys.onReturnPressed: card.choose(tile.s)
                Keys.onEnterPressed: card.choose(tile.s)
                onActiveFocusChanged: { if (activeFocus) treemap.focused = tile.s.id; else if (treemap.focused === tile.s.id) treemap.focused = "" }
                Rectangle { visible: tile.activeFocus; anchors.fill: parent; radius: tile.radius; color: "transparent"; border.width: 2; border.color: Theme.accent }
                Accessible.role: Accessible.Button
                Accessible.name: card.spokenDetail(s)
                Accessible.onPressAction: card.choose(tile.s)
            }
        }
        // the detail card in the bottom right corner
        Rectangle {
            visible: treemap.focusSlice !== null
            anchors.right: parent.right; anchors.bottom: parent.bottom; anchors.margins: 8
            width: Math.min(detailCol.implicitWidth + 20, 260 * Theme.textScale); height: detailCol.implicitHeight + 20
            radius: 8; color: Theme.card; border.color: Theme.border
            ColumnLayout {
                id: detailCol
                x: 10; y: 10
                width: parent.width - 20
                spacing: 3
                Text { text: treemap.focusSlice ? treemap.focusSlice.label : ""; textFormat: Text.PlainText; Layout.fillWidth: true; wrapMode: Text.WordWrap; maximumLineCount: 2
                       font.pixelSize: Theme.subheadline + 1; font.weight: Font.DemiBold; color: Theme.text
                       Sensitive { labels: !(treemap.focusSlice && treemap.focusSlice.personal) } }
                Text { text: treemap.focusSlice ? Fmt.money(treemap.focusSlice.cents, card.currency) : ""; font.pixelSize: Theme.callout + 1; font.features: { "tnum": 1 }; color: Theme.text
                       Sensitive { amounts: true } }
                Text { text: treemap.focusSlice ? qsTr("Anteil %1").arg(card.shareText(treemap.focusSlice.cents)) : ""; font.pixelSize: Theme.caption; font.features: { "tnum": 1 }; color: Theme.secondary
                       Sensitive { amounts: true } }
                Text { visible: !!treemap.focusSlice && treemap.focusSlice.count > 1; text: treemap.focusSlice ? qsTr("%1 Positionen").arg(treemap.focusSlice.count) : ""
                       font.pixelSize: Theme.caption2; color: Theme.tertiary }
            }
        }
        Sensitive {}
    }

    IconButton {
        visible: card.total > 0 && card.filter !== null
        symbol: "xmark.circle.fill"; symbolSize: 12; tint: Theme.accent
        text: qsTr("Alle anzeigen"); textColor: Theme.accent; font.pixelSize: Theme.caption
        onClicked: card.filterRequested(null)
    }
    ColumnLayout { id: footerSlot; Layout.fillWidth: true; spacing: 4 }

    /// The focus ring of a row-like control. A layout would place a frame of its own as a cell, so it sits in the
    /// row's first item and spans the whole `target` from there.
    component FocusFrame: Rectangle {
        required property Item target
        visible: target.activeFocus
        x: -parent.x - 3; y: -parent.y - 2
        width: target.width + 6; height: target.height + 4
        radius: 4; color: "transparent"; border.width: 2; border.color: Theme.accent
    }

    /// Name, amount and share per slice; a long name wraps to a second line, complete in the tooltip.
    component Legend: ColumnLayout {
        id: legend
        property var slices: []
        spacing: 6
        Repeater {
            model: legend.slices
            RowLayout {
                id: entry
                required property var modelData
                Layout.fillWidth: true
                spacing: 8
                opacity: card.selected === "" || card.selected === modelData.id ? 1 : 0.35
                Rectangle { width: 9; height: 9; radius: 4.5; color: entry.modelData.color; Layout.alignment: Qt.AlignVCenter
                            FocusFrame { target: entry } }
                Text { text: entry.modelData.label; textFormat: Text.PlainText; Layout.fillWidth: true; wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight
                       font.pixelSize: Theme.caption; color: entry.modelData.unclassified ? Theme.secondary : Theme.text
                       Sensitive { labels: !entry.modelData.personal } }
                Text { text: Fmt.money(entry.modelData.cents, card.currency); font.pixelSize: Theme.caption; font.features: { "tnum": 1 }; color: Theme.secondary
                       Sensitive { amounts: true } }
                Text { text: card.shareText(entry.modelData.cents); Layout.minimumWidth: 46 * Theme.textScale; horizontalAlignment: Text.AlignRight
                       font.pixelSize: Theme.caption; font.features: { "tnum": 1 }; color: Theme.secondary; Sensitive { amounts: true } }
                TapHandler { onTapped: card.choose(entry.modelData) }
                HoverHandler { id: legendArea; cursorShape: Qt.PointingHandCursor }
                ToolTip.visible: legendArea.hovered; ToolTip.delay: 600; ToolTip.text: card.spokenDetail(entry.modelData)
                // a Button on the Mac: Tab reaches it, Space / Return filters
                activeFocusOnTab: true
                Keys.onSpacePressed: card.choose(entry.modelData)
                Keys.onReturnPressed: card.choose(entry.modelData)
                Keys.onEnterPressed: card.choose(entry.modelData)
                Accessible.role: Accessible.Button
                Accessible.name: card.spokenLabel(entry.modelData)
                Accessible.checked: card.selected === entry.modelData.id
                Accessible.description: qsTr("Positionen dieser Gruppe anzeigen oder Filter aufheben")
                Accessible.onPressAction: card.choose(entry.modelData)
            }
        }
    }
}
