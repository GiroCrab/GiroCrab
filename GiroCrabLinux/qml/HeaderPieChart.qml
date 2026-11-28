import QtQuick
import QtQuick.Layouts

// HeaderChartControls.swift › HeaderPieChart, HeaderPieSlices, AccountExpensePie (#634):
// the header chart as a donut with its legend next to it. A category shows the shares of its
// subcategories (plus the bookings directly in it; a legend click opens the subcategory), an
// account, group or all accounts the expenses by top-level category in the selected period
// (one SQL aggregate, loaded off the GUI thread).
Item {
    id: pie
    readonly property bool isCategory: AppModel.sidebarSelection.startsWith("category:")
    /// HeaderPieSlices.limit (macOS)
    readonly property int limit: 9
    /// HeaderPieChart.rows (macOS): entries per legend column
    readonly property int rows: 3

    /// AccountExpensePie.loaded: the account pie's parts ({ id, label, cents }) and the accounts
    /// and period they were loaded for. Shown only while those match (Key.covers): an older
    /// load of the same selection may stand in, another selection's never, also not after a
    /// failed load.
    property var loaded: null
    readonly property string cover: isCategory ? "" : JSON.stringify([AppModel.historyRequest().ids, AppModel.periodRange()])
    readonly property bool expensesShown: loaded !== null && loaded.cover === cover
    readonly property var parts: isCategory ? subcategories(Number(AppModel.sidebarSelection.slice(9)), AppModel.categories)
        : expensesShown ? loaded.parts : []
    readonly property var slices: fold(parts, limit)
    readonly property real total: slices.reduce((t, s) => t + s.cents, 0)
    readonly property string emptyText: isCategory ? qsTr("Keine Buchungen im Zeitraum") : qsTr("Keine kategorisierten Ausgaben im Zeitraum")

    /// HeaderPieSlices.subcategories(of:in:): direct subcategories (incl. their own children)
    /// plus the bookings directly in the category. Only parts on the side of the category's
    /// total count, so a refund in an expense category does not show as a share.
    function subcategories(id, categories) {
        const parent = categories.find(c => c.id === id)
        if (!parent) return []
        const sign = parent.total_cents < 0 ? -1 : 1
        const out = categories.filter(c => c.parent_id === id).map(c => ({ id: String(c.id), label: c.name, cents: c.total_cents * sign }))
        out.push({ id: "own", label: qsTr("Direkt: %1").arg(parent.name), cents: parent.own_total_cents * sign })
        return out.filter(p => p.cents > 0)
    }
    /// HeaderPieSlices.expenses: top-level categories (totals include the subcategories).
    function expenses(categories) {
        return categories.filter(c => c.depth === 0 && c.total_cents < 0).map(c => ({ id: String(c.id), label: c.name, cents: -c.total_cents }))
    }
    /// HeaderPieSlices.fold: largest first; beyond `limit` slices the rest is folded into „Sonstige“.
    function fold(all, max) {
        const sorted = all.slice().sort((a, b) => a.cents !== b.cents ? b.cents - a.cents : a.label.localeCompare(b.label))
        const head = sorted.length > max ? sorted.slice(0, Math.max(max - 1, 0)) : sorted
        const out = head.map((p, i) => ({ id: p.id, label: p.label, cents: p.cents, color: hue(i) }))
        const rest = sorted.slice(head.length).reduce((t, p) => t + p.cents, 0)
        if (rest > 0) out.push({ id: "rest", label: qsTr("Sonstige"), cents: rest, color: restColor })
        return out
    }
    // WealthView.swift › WealthPalette: distinct, colour-blind friendly hues (index wraps)
    readonly property var hues: [Theme.c("#2F6FDB", "#6EA0F5"), Theme.c("#1F7A5C", "#5EC79F"), Theme.c("#D98A00", "#F5B94A"),
                                 Theme.c("#8E4FC4", "#B98AE6"), Theme.c("#D1495B", "#F27F8E"), Theme.c("#0E8FA8", "#5CC6DB"),
                                 Theme.c("#7C9A2D", "#A9C95C"), Theme.c("#B0568F", "#DB8FC0")]
    function hue(i) { return hues[i % hues.length] }
    readonly property color restColor: Theme.c("#B4BEC8", "#6B7785")
    /// .percent.precision(.fractionLength(0))
    function percent(x) { const n = Math.round(x * 100); return Fmt.english ? n + "%" : n + " %" }

    // PieHover.swift (#889): the slice under the pointer (or the hovered legend entry) stands
    // out, the others fade; a card at the pointer shows name, amount and share.
    /// `hovered`: id of the highlighted slice, "" for none.
    property string hovered: ""
    /// `spot`: the pointer in this item's coordinates while it is over a slice or legend entry.
    property point spot
    readonly property var hoveredSlice: hovered === "" ? null : slices.find(s => s.id === hovered) || null
    onHoveredChanged: canvas.requestPaint()
    onVisibleChanged: if (!visible) hovered = ""
    /// PieValueMode.shareText: share of the total with one digit, a dash without a positive total.
    function shareText(cents) { return total > 0 ? DepotMetrics.percent(cents / total * 100, false, 1) : "–" }
    /// PieHit.id(at:in:innerRatio:parts:): the slice at `x`/`y` of the canvas, "" outside the ring.
    function hit(x, y) {
        const dx = x - canvas.width / 2, dy = y - canvas.height / 2
        const distance = Math.hypot(dx, dy), radius = Math.min(canvas.width, canvas.height) / 2
        if (radius <= 0 || total <= 0 || distance < radius * 0.55 || distance > radius) return ""
        let angle = Math.atan2(dx, -dy)   // clockwise from twelve o'clock, like the slices are drawn
        if (angle < 0) angle += 2 * Math.PI
        const v = angle / (2 * Math.PI) * total
        let end = 0
        for (const p of slices) {
            if (p.cents <= 0) continue
            end += p.cents
            if (v < end) return p.id
        }
        return ""
    }

    // AccountExpensePie: reloads when the accounts, the period or the category sums change.
    readonly property string loadKey: isCategory || !visible ? ""
        : JSON.stringify([AppModel.historyRequest().ids, AppModel.periodRange(), AppModel.categories.map(c => c.total_cents)])
    onLoadKeyChanged: if (loadKey !== "") Qt.callLater(load)
    function load() {
        if (loadKey === "" || !AppModel.isUnlocked) return
        let cover = ""
        AppModel.loadOffMain("headerPie", () => {
            const r = AppModel.periodRange(), ids = AppModel.historyRequest().ids
            cover = JSON.stringify([ids, r])
            return [["list_categories_filtered", { date_from: r[0], date_to: r[1], account_ids: ids,
                                                   checked: null, direction: "Expense" }]]
        }, r => { pie.loaded = { cover: cover, parts: pie.expenses(r[0]) } }, () => {})
    }

    // .contextMenu { ChartStyleInlinePicker() }
    MouseArea { anchors.fill: parent; acceptedButtons: Qt.RightButton; onClicked: styleMenu.popup() }
    AppMenu {
        id: styleMenu
        Repeater {
            model: BalanceChartPrefs.styles
            AppMenuItem { required property var modelData; text: modelData.title; checkable: true
                          checked: BalanceChartPrefs.style === modelData.id
                          onTriggered: { checked = Qt.binding(() => BalanceChartPrefs.style === modelData.id); BalanceChartPrefs.setStyle(modelData.id) } }
        }
    }

    onSlicesChanged: canvas.requestPaint()
    Connections { target: Theme; function onDarkChanged() { canvas.requestPaint() } }

    Text {
        visible: pie.total <= 0 && (pie.isCategory || pie.expensesShown)
        anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
        text: pie.emptyText; font.pixelSize: Theme.caption; color: Theme.secondary
    }
    RowLayout {
        visible: pie.total > 0
        anchors.fill: parent
        spacing: 14
        Accessible.role: Accessible.Chart; Accessible.name: qsTr("Anteile")
        // SectorMark(innerRadius: .ratio(0.55), angularInset: 1).cornerRadius(2)
        Canvas {
            id: canvas
            Layout.preferredWidth: pie.height; Layout.fillHeight: true
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                const sum = pie.total
                if (sum <= 0) return
                const cx = width / 2, cy = height / 2, r = Math.min(width, height) / 2, inner = r * 0.55
                const inset = pie.slices.length > 1 ? 1 : 0
                let a = -Math.PI / 2
                for (const p of pie.slices) {
                    ctx.globalAlpha = pie.hovered === "" || pie.hovered === p.id ? 1 : 0.35
                    const span = p.cents / sum * 2 * Math.PI
                    const o = inset / r, i = inset / inner
                    if (span > 2 * o) {
                        ctx.beginPath()
                        ctx.arc(cx, cy, r, a + o, a + span - o, false)
                        ctx.arc(cx, cy, inner, a + span - Math.min(i, span / 2), a + Math.min(i, span / 2), true)
                        ctx.closePath()
                        ctx.fillStyle = p.color
                        ctx.fill()
                    }
                    a += span
                }
            }
            // .pieHover: the slice under the pointer, the card where the pointer is
            HoverHandler {
                onPointChanged: {
                    if (!hovered) return
                    const id = pie.hit(point.position.x, point.position.y)
                    if (pie.hovered !== id) pie.hovered = id
                    if (id !== "") pie.spot = canvas.mapToItem(pie, point.position.x, point.position.y)
                }
                onHoveredChanged: if (!hovered) pie.hovered = ""
            }
        }
        // legend: columns of `rows` entries, each at most 200 pt wide
        Row {
            Layout.alignment: Qt.AlignVCenter
            spacing: 16
            Repeater {
                model: Math.ceil(pie.slices.length / pie.rows)
                Column {
                    id: column
                    required property int index
                    spacing: 2
                    Repeater {
                        model: pie.slices.slice(column.index * pie.rows, column.index * pie.rows + pie.rows)
                        Item {
                            id: entry
                            required property var modelData
                            readonly property bool selectable: pie.isCategory && /^\d+$/.test(modelData.id)
                            width: Math.ceil(row.implicitWidth); height: row.implicitHeight
                            implicitWidth: width
                            // a column is at most 200 pt wide: the name truncates, the share never
                            RowLayout {
                                id: row
                                spacing: 5
                                Rectangle { width: 7; height: 7; radius: 3.5; color: entry.modelData.color }
                                Text { text: entry.modelData.label; font.pixelSize: Theme.caption2; color: Theme.text
                                       elide: Text.ElideRight
                                       Layout.preferredWidth: Math.ceil(Math.min(implicitWidth, 200 * Theme.textScale - 12 - share.implicitWidth))
                                       Sensitive { labels: true } }
                                // PieValueMode: the share (default) or the amount
                                Text { id: share; text: BalanceChartPrefs.pieValueMode === "amount" ? Fmt.money(entry.modelData.cents)
                                                                                                    : pie.percent(entry.modelData.cents / pie.total)
                                       font.pixelSize: Theme.caption2
                                       font.features: { "tnum": 1 }; color: Theme.secondary
                                       Sensitive { amounts: true } }
                            }
                            opacity: pie.hovered === "" || pie.hovered === entry.modelData.id ? 1 : 0.5
                            // .sliceHover: the entry highlights its slice and shows the card at the pointer
                            HoverHandler {
                                onPointChanged: if (hovered) {
                                    pie.hovered = entry.modelData.id
                                    pie.spot = entry.mapToItem(pie, point.position.x, point.position.y)
                                }
                                onHoveredChanged: if (!hovered && pie.hovered === entry.modelData.id) pie.hovered = ""
                            }
                            // a legend click opens the subcategory (category slices only)
                            TapHandler { enabled: entry.selectable; cursorShape: Qt.PointingHandCursor
                                         onTapped: AppModel.sidebarSelection = "category:" + entry.modelData.id }
                            Accessible.role: entry.selectable ? Accessible.Button : Accessible.StaticText
                            Accessible.name: AppModel.masking(entry.modelData.label, AppModel.redactLabels)
                            Accessible.description: AppModel.masking(Fmt.money(entry.modelData.cents) + " · " + pie.shareText(entry.modelData.cents), AppModel.redactAmounts)
                        }
                    }
                }
            }
        }
        Item { Layout.fillWidth: true }
    }
    // PieDetailCard: name, amount and share of the hovered slice
    HoverCard {
        source: pie
        point: pie.spot
        shown: pie.hoveredSlice !== null && pie.total > 0
        Text { text: pie.hoveredSlice ? pie.hoveredSlice.label : ""; font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.text
               width: Math.min(implicitWidth, 184 * Theme.textScale); wrapMode: Text.Wrap; maximumLineCount: 2; elide: Text.ElideRight
               Sensitive { labels: true } }
        Text { text: pie.hoveredSlice ? Fmt.money(pie.hoveredSlice.cents) : ""; font.pixelSize: Theme.caption; font.features: { "tnum": 1 }; color: Theme.text
               Sensitive { amounts: true } }
        Text { text: pie.hoveredSlice ? pie.shareText(pie.hoveredSlice.cents) : ""; font.pixelSize: Theme.caption2; font.features: { "tnum": 1 }; color: Theme.secondary
               Sensitive { amounts: true } }
    }
}
