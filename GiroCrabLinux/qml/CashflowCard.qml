import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtCore

// PlanningUI.swift › CashflowCard: income/expense bars per month, quarter or year.
// Folds to its title row with the net of the latest period (GitHub #603).
Card {
    id: card
    // ViewLayout.cardPadding / cardVerticalPadding(collapsed:)
    padding: 12
    verticalPadding: collapsed ? 4 : 12
    /// `@AppStorage(ViewLayout.cashflowCollapsedKey)`
    readonly property bool collapsed: layout.cashflowCollapsed
    property Settings layout: Settings { property bool cashflowCollapsed: false }
    property string granularity: "Month"
    /// @AppStorage("chartColorPalette")
    readonly property string paletteName: AppModel.uiDefaults.chartColorPalette
    property var rows: []
    onRowsChanged: hovered = -1
    /// Period under the pointer (index into `rows`, -1 for none) and where the pointer is (#889).
    property int hovered: -1
    property point spot
    readonly property var hoveredRow: hovered >= 0 && hovered < rows.length ? rows[hovered] : null
    readonly property var hexes: ({
        girocrab: { income: ["#38BDF8", "#0284C7"], expense: ["#F97316", "#EA580C"] },
        profi: { income: ["#3B82F6", "#2563EB"], expense: ["#64748B", "#475569"] },
        emerald: { income: ["#10B981", "#059669"], expense: ["#F43F5E", "#E11D48"] } })[paletteName]
    /// Off the GUI thread: every published booking list asks again (a load still running
    /// would otherwise hold the GUI thread at the vault mutex).
    function load() {
        // hidden (Auswertungen off, budgets alone above the list, #544): nothing to load
        if (!AppModel.isUnlocked || !visible) return
        AppModel.loadOffMain("cashflow", () => [["cashflow_by_period", { granularity: card.granularity, count: card.granularity === "Year" ? 5 : 6 }]],
                             r => { card.rows = r[0] })
    }
    onGranularityChanged: load()
    onVisibleChanged: if (visible) load()
    Connections { target: AppModel; function onBookingsChanged() { card.load() } }
    Component.onCompleted: load()
    function label(key) {
        if (granularity !== "Month") return key
        const d = Fmt.date(key + "-01")
        return d ? d.toLocaleDateString(Qt.locale(), "MMM yy") : key
    }
    readonly property real maxValue: {
        let m = 0
        for (const r of rows) m = Math.max(m, Math.abs(r.income_cents), Math.abs(r.expense_cents))
        if (m === 0) return 100
        const step = Math.pow(10, Math.floor(Math.log10(m / 100))) * 100
        return Math.ceil(m / step / 2) * step * 2
    }

    ColumnLayout {
        width: parent.width
        spacing: 8   // ViewLayout.cardSpacing
        RowLayout {
            Layout.fillWidth: true
            CardChevron { collapsed: card.collapsed; onToggled: card.layout.cashflowCollapsed = !card.collapsed }
            Text { text: qsTr("Cashflow"); font.pixelSize: Theme.subheadline + 1; font.weight: Font.DemiBold; color: Theme.text; Layout.fillWidth: true }
            // folded: the latest period's net
            Text {
                readonly property var last: card.rows.length ? card.rows[card.rows.length - 1] : null
                readonly property real net: last ? last.income_cents - last.expense_cents : 0
                visible: card.collapsed && last !== null
                text: last ? card.label(last.key) + ": " + Fmt.money(net) : ""
                font.pixelSize: Theme.subheadline + 1; font.weight: Font.DemiBold; font.features: { "tnum": 1 }
                color: net >= 0 ? Theme.inflow : Theme.outflow
                Sensitive { amounts: true }
            }
            IconButton { visible: !card.collapsed; symbol: "paintpalette"; symbolSize: 14;
                ToolTip.text: qsTr("Farbschema"); ToolTip.visible: hovered
                onClicked: paletteMenu.popup()
                AppMenu {
                    id: paletteMenu
                    Repeater {
                        model: [{ id: "girocrab", title: qsTr("GiroCrab — Cyan & Orange") }, { id: "profi", title: qsTr("Profi-Banking — Blau & Schiefer") },
                                { id: "emerald", title: qsTr("Klassisch — Smaragd & Rose") }]
                        AppMenuItem { required property var modelData; text: modelData.title; checkable: true; checked: card.paletteName === modelData.id; onTriggered: AppModel.uiDefaults.chartColorPalette = modelData.id }
                    }
                }
            }
            Segmented {
                visible: !card.collapsed
                Layout.preferredWidth: 220
                options: [{ id: "Month", title: qsTr("Monat") }, { id: "Quarter", title: qsTr("Quartal") }, { id: "Year", title: qsTr("Jahr") }]
                Accessible.name: qsTr("Zeitraum")   // Picker("Zeitraum", …).labelsHidden() (ff91866)
                current: card.granularity
                onActivated: id => card.granularity = id
            }
        }
        Item {
            visible: !card.collapsed
            Layout.fillWidth: true
            Layout.preferredHeight: 160
            // PlanningUI.swift › CashflowCard: .accessibilityLabel("Cashflow") + chart descriptor (#571)
            Accessible.role: Accessible.Chart
            Accessible.name: qsTr("Cashflow")
            Accessible.description: qsTr("Einnahmen und Ausgaben je Zeitraum")
            // y axis + dashed grid
            Repeater {
                model: 4
                Item {
                    required property int index
                    readonly property real v: card.maxValue * (3 - index) / 3
                    y: index * (140 / 3); width: parent.width; height: 1
                    Text { text: Number(v / 100).toLocaleString(Qt.locale(), "f", 0); font.pixelSize: Theme.caption; color: Theme.secondary; anchors.verticalCenter: parent.verticalCenter
                           Sensitive { amounts: true } }   // chart axis values
                    Row {
                        x: 44; width: parent.width - 44; spacing: 3; clip: true
                        Repeater { model: Math.max(0, Math.ceil(parent.width / 6)); Rectangle { width: 3; height: 0.5; color: Theme.border } }
                    }
                }
            }
            Row {
                x: 44; width: parent.width - 44; height: 140
                Repeater {
                    model: card.rows
                    Item {
                        id: period
                        required property var modelData
                        required property int index
                        width: parent.width / Math.max(1, card.rows.length); height: 140
                        HoverHandler {
                            onPointChanged: if (hovered) {
                                card.hovered = period.index
                                card.spot = period.mapToItem(card, point.position.x, point.position.y)
                            }
                            onHoveredChanged: if (!hovered && card.hovered === period.index) card.hovered = -1
                        }
                        Row {
                            anchors.bottom: parent.bottom; anchors.horizontalCenter: parent.horizontalCenter
                            spacing: width / 8
                            // the other periods' bars fade while one is hovered
                            opacity: card.hovered < 0 || card.hovered === period.index ? 1 : 0.4
                            Repeater {
                                model: [{ v: modelData.income_cents, c: card.hexes.income }, { v: Math.abs(modelData.expense_cents), c: card.hexes.expense }]
                                Rectangle {
                                    required property var modelData
                                    anchors.bottom: parent.bottom
                                    width: Math.min(46, parent.parent.width / 4)
                                    height: Math.max(0, modelData.v / card.maxValue * 140)
                                    radius: 5
                                    gradient: Gradient {
                                        GradientStop { position: 0; color: Theme.withAlpha(modelData.c[0], 0.95) }
                                        GradientStop { position: 1; color: Theme.withAlpha(modelData.c[1], 0.45) }
                                    }
                                }
                            }
                        }
                        Text { anchors.horizontalCenter: parent.horizontalCenter; y: 144; text: card.label(modelData.key); font.pixelSize: Theme.caption; color: Theme.secondary }
                    }
                }
            }
            // hoverCard: income, expenses and the balance of one period, at the pointer
            HoverCard {
                id: periodCard
                readonly property var r: card.hoveredRow
                readonly property real net: r ? r.income_cents - r.expense_cents : 0
                source: card
                point: card.spot
                shown: r !== null && !card.collapsed
                minimumWidth: 150
                Text { text: periodCard.r ? card.label(periodCard.r.key) : ""; font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.text }
                Repeater {
                    model: periodCard.r ? [{ t: qsTr("Einnahmen"), v: periodCard.r.income_cents, c: Theme.inflow },
                                           { t: qsTr("Ausgaben"), v: periodCard.r.expense_cents, c: Theme.outflow },
                                           { t: qsTr("Saldo"), v: periodCard.net, c: periodCard.net >= 0 ? Theme.inflow : Theme.outflow }] : []
                    Item {
                        required property var modelData
                        width: Math.max(periodCard.minimumWidth, implicitWidth)
                        implicitWidth: lineTitle.implicitWidth + 10 + lineAmount.implicitWidth
                        height: Math.max(lineTitle.implicitHeight, lineAmount.implicitHeight)
                        Text { id: lineTitle; text: modelData.t; font.pixelSize: Theme.caption; color: Theme.secondary }
                        Text { id: lineAmount; anchors.right: parent.right; text: Fmt.money(modelData.v); font.pixelSize: Theme.caption
                               font.features: { "tnum": 1 }; color: modelData.c
                               Sensitive { amounts: true } }
                    }
                }
            }
        }
        RowLayout {
            visible: !card.collapsed
            spacing: 14
            Repeater {
                model: [{ c: card.hexes.income[0], t: qsTr("Einnahmen") }, { c: card.hexes.expense[0], t: qsTr("Ausgaben") }]
                RowLayout { required property var modelData; spacing: 5
                    Rectangle { width: 8; height: 8; radius: 4; color: modelData.c }
                    Text { text: modelData.t; font.pixelSize: Theme.caption; color: Theme.secondary } }
            }
        }
    }
}
