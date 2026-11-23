import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// DepotPositionsTable.swift (#564, #625): the positions of a holdings view – a compact sortable
// table in wide windows, two-line rows with unfoldable details in narrow ones. Shares are relative
// to the whole inventory, so a search changes the rows but never the percentages. One table for the
// depot, the Sachwerte account and the Vermögen tabs; a view that lets rows be selected or opened
// sets `selectable` and handles `selected` / `opened`, and adds its own menu entries (`rowActions`).
Rectangle {
    id: table
    /// DepotPosition list (DepotMetrics.makePositions / mergedPositions / tangiblePositions), sorted and filtered.
    property var positions: []
    /// Positions of the whole inventory (the rows shown may be a search result).
    property int totalCount: 0
    /// DepotSort { key, ascending }; the host owns it (`sortRequested`).
    property var sort: ({ key: "value", ascending: false })
    signal sortRequested(var sort)
    property bool wide: true
    property string currency: "EUR"
    /// Securities without a bank figure get one from the cost price: the table says so (not Sachwerte).
    property bool notesCalculatedGain: true
    property string selectedID: ""
    property bool selectable: false
    signal selected(var position)
    signal opened(var position)
    /// RowTaps (macOS): a double click first – a single click may leave the view, so it waits
    /// out the double-click interval before `selected`.
    Timer {
        id: clickTimer
        property var position: null
        interval: Qt.styleHints.mouseDoubleClickInterval
        onTriggered: { if (table.visible) table.selected(position) }
    }
    // a scope switch within the interval hides the table: the pending click no longer selects
    onVisibleChanged: if (!visible) clickTimer.stop()
    signal copyAll()
    /// Extra context-menu entries of the host: position => [{ title, action, separator }]
    property var rowActions: null
    /// valueHistory(available:open:) (#578): positions with a value history get a chart button and
    /// a menu item that emit `historyRequested`. null: no history column.
    property var historyAvailable: null
    signal historyRequested(var position)
    function hasHistory(p) { return !!historyAvailable && !!p && historyAvailable(p) }
    /// Which depot(s) hold a position, where the table spans several (Vermögen).
    property var depotLabel: null
    /// Rows unfolded in the narrow layout (`--expand-rows`: the first two).
    property var expanded: ({})
    /// `--expand-rows`: the first two rows start unfolded, once there are rows (screenshots).
    property bool expandedOnce: false
    onPositionsChanged: {
        if (expandedOnce || positions.length === 0 || AppModel.launchArgs.indexOf("--expand-rows") < 0) return
        expandedOnce = true
        const e = {}
        for (const p of positions.slice(0, 2)) e[p.id] = true
        expanded = e
    }

    readonly property real qtyW: 88 * Theme.textScale
    readonly property real priceW: 88 * Theme.textScale
    readonly property real valueW: 104 * Theme.textScale
    readonly property real shareW: 64 * Theme.textScale
    readonly property real gainW: 124 * Theme.textScale
    readonly property real editW: (historyAvailable ? 44 : 22) * Theme.textScale
    readonly property bool anyCalculatedGain: positions.some(p => p.gain && !p.gain.reported)

    implicitHeight: col.implicitHeight
    radius: 12
    color: Theme.card
    border.color: Theme.border
    clip: true

    // MARK: texts
    function quantityText(p) { return NumberText.format(p.holding.quantity) }
    function unitText(p) { return p.basis === "percentOfNominal" ? qsTr("Nominal") : qsTr("Stück") }
    function unitShort(p) { return p.basis === "percentOfNominal" ? qsTr("Nominal") : qsTr("Stk.") }
    /// Bonds quote a percentage of the nominal amount; the price in the position's own currency (#625).
    function priceText(p) {
        if (p.hidesPrice) return "–"
        if (p.basis === "percentOfNominal") return DepotMetrics.percent(p.holding.current_price_cents / 100)
        const trade = DepotMetrics.nativeCurrency(p.holding, currency)
        if (trade !== null && DepotMetrics.has(p.holding.native_price_cents)) return Fmt.money(p.holding.native_price_cents, trade)
        return Fmt.money(p.holding.current_price_cents, currency)
    }
    function valueText(p) {
        const trade = DepotMetrics.nativeCurrency(p.holding, currency)
        if (trade !== null && DepotMetrics.has(p.holding.native_value_cents)) return Fmt.money(p.holding.native_value_cents, trade)
        return Fmt.money(p.holding.value_cents, currency)
    }
    function shareText(p) { return p.share !== null ? DepotMetrics.percent(p.share * 100, false, 1) : "–" }
    function gainText(p) {
        return p.gain ? [DepotMetrics.signedMoney(p.gain.cents, currency), p.gain.percent !== null ? DepotMetrics.percent(p.gain.percent, true) : null].filter(x => x).join(" · ") : ""
    }
    /// "160 Stk. · 104,25 € · Aktien · ETF"
    function narrowSubline(p) {
        const c = AssetClassification.compactTitle(p.asset.classification)
        return [quantityText(p) + " " + unitShort(p), p.hidesPrice ? null : priceText(p), c === "" ? null : c].filter(x => x).join(" · ")
    }
    function classify(p) { AssetClassification.edit(p.holding.name, p.asset.classification) }
    function toggle(id) { const e = Object.assign({}, expanded); if (e[id]) delete e[id]; else e[id] = true; expanded = e }

    ColumnLayout {
        id: col
        width: parent.width
        spacing: 0

        // MARK: header
        RowLayout {
            visible: table.wide
            Layout.fillWidth: true; Layout.leftMargin: 14; Layout.rightMargin: 14; Layout.topMargin: 8; Layout.bottomMargin: 8
            spacing: 10
            SortHeader { title: qsTr("Position"); key: "name"; Layout.fillWidth: true; Layout.preferredWidth: 1 }
            HeadLabel { title: qsTr("Menge / Einheit"); Layout.preferredWidth: table.qtyW }
            HeadLabel { title: qsTr("Kurs"); Layout.preferredWidth: table.priceW }
            SortHeader { title: qsTr("Marktwert"); key: "value"; trailing: true; Layout.preferredWidth: table.valueW }
            HeadLabel { title: qsTr("Anteil"); Layout.preferredWidth: table.shareW }
            SortHeader { title: qsTr("GuV € / %"); key: "gain"; trailing: true; Layout.preferredWidth: table.gainW }
            Item { Layout.preferredWidth: table.editW }
        }
        RowLayout {
            visible: !table.wide
            Layout.fillWidth: true; Layout.leftMargin: 14; Layout.rightMargin: 14; Layout.topMargin: 8; Layout.bottomMargin: 8
            spacing: 6
            Text { text: qsTr("Sortierung"); font.pixelSize: Theme.caption; color: Theme.secondary }
            Item {
                id: sortButton
                implicitWidth: sortRow.implicitWidth; implicitHeight: sortRow.implicitHeight
                // a Menu on the Mac: Tab reaches it, Space / Return opens it
                activeFocusOnTab: true
                Keys.onSpacePressed: sortMenu.popup()
                Keys.onReturnPressed: sortMenu.popup()
                Keys.onEnterPressed: sortMenu.popup()
                Rectangle { visible: sortButton.activeFocus; anchors.fill: parent; anchors.margins: -2
                            radius: 4; color: "transparent"; border.width: 2; border.color: Theme.accent }
                Accessible.role: Accessible.ButtonMenu
                Accessible.name: qsTr("Sortieren nach")
                Accessible.onPressAction: sortMenu.popup()
                RowLayout { id: sortRow; spacing: 4
                    SFIcon { symbol: "chevron.down"; rotation: table.sort.ascending ? 180 : 0; size: 9; color: Theme.text }
                    Text { text: table.sort.key === "gain" ? qsTr("GuV") : table.sort.key === "name" ? qsTr("Name") : qsTr("Marktwert")
                           font.pixelSize: Theme.caption; font.weight: Font.Medium; color: Theme.text } }
                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: sortMenu.popup() }
                AppMenu {
                    id: sortMenu
                    AppMenuSeparator { contentItem: Text { text: qsTr("Sortieren nach"); font.pixelSize: Theme.caption; color: Theme.secondary; leftPadding: 8 } }
                    Repeater {
                        model: [{ id: "value", title: qsTr("Marktwert") }, { id: "gain", title: qsTr("GuV") }, { id: "name", title: qsTr("Name") }]
                        AppMenuItem { required property var modelData; text: modelData.title; checkable: true; checked: table.sort.key === modelData.id
                                      onTriggered: table.sortRequested({ key: modelData.id, ascending: modelData.id === "name" }) }
                    }
                    AppMenuSeparator {}
                    AppMenuItem { text: qsTr("Aufsteigend"); checkable: true; checked: table.sort.ascending
                                  onTriggered: table.sortRequested({ key: table.sort.key, ascending: !table.sort.ascending }) }
                }
            }
            Item { Layout.fillWidth: true }
        }
        Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: Theme.border }

        // MARK: rows
        Repeater {
            model: table.positions
            ColumnLayout {
                id: rowBox
                required property var modelData
                required property int index
                readonly property var p: modelData
                readonly property bool open: !!table.expanded[p.id]
                Layout.fillWidth: true
                spacing: 0
                Rectangle {
                    id: rowArea
                    Layout.fillWidth: true
                    implicitHeight: (table.wide ? Math.max(44, wideRow.implicitHeight) : Math.max(44, narrowRow.implicitHeight) + (rowBox.open ? details.implicitHeight + 14 : 0)) + 10
                    color: table.selectedID !== "" && table.selectedID === rowBox.p.id ? Theme.selection : "transparent"
                    // a selectable row is a button for the keyboard too: Tab reaches it, Space selects,
                    // Return / Enter opens like the double click
                    activeFocusOnTab: table.selectable
                    Keys.onSpacePressed: table.selected(rowBox.p)
                    Keys.onReturnPressed: table.opened(rowBox.p)
                    Keys.onEnterPressed: table.opened(rowBox.p)
                    Accessible.role: table.selectable ? Accessible.Button : Accessible.Row
                    Accessible.name: AppModel.masking(rowBox.p.displayName, rowBox.p.isOwnItem ? undefined : AppModel.redactLabels)
                    Accessible.onPressAction: { if (table.selectable) table.opened(rowBox.p) }
                    Rectangle { visible: rowArea.activeFocus; anchors.fill: parent; anchors.margins: 1
                                radius: 4; color: "transparent"; border.width: 2; border.color: Theme.accent }
                    MouseArea {
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                        onClicked: m => {
                            if (m.button === Qt.RightButton) { rowMenu.position = rowBox.p; rowMenu.rebuild(); rowMenu.popup(); return }
                            if (table.selectable) { clickTimer.position = rowBox.p; clickTimer.restart() }
                        }
                        onDoubleClicked: m => {
                            if (m.button !== Qt.LeftButton || !table.selectable) return
                            clickTimer.stop()
                            table.opened(rowBox.p)
                        }
                    }
                    // wide: the columns
                    RowLayout {
                        id: wideRow
                        visible: table.wide
                        anchors.left: parent.left; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                        anchors.leftMargin: 14; anchors.rightMargin: 14
                        spacing: 10
                        NameBlock { p: rowBox.p; Layout.fillWidth: true; Layout.preferredWidth: 1 }
                        ColumnLayout { spacing: 1; Layout.preferredWidth: table.qtyW
                            Text { Layout.alignment: Qt.AlignRight; text: table.quantityText(rowBox.p); font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: Theme.text
                                   Sensitive { amounts: true } }
                            Text { Layout.alignment: Qt.AlignRight; text: table.unitText(rowBox.p); font.pixelSize: Theme.caption2; color: Theme.tertiary } }
                        Text { Layout.preferredWidth: table.priceW; horizontalAlignment: Text.AlignRight; elide: Text.ElideLeft
                               text: table.priceText(rowBox.p); font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: Theme.text
                               Sensitive { amounts: true } }
                        Text { Layout.preferredWidth: table.valueW; horizontalAlignment: Text.AlignRight; elide: Text.ElideLeft
                               text: table.valueText(rowBox.p); font.pixelSize: Theme.body; font.weight: Font.Medium; font.features: { "tnum": 1 }; color: Theme.text
                               Sensitive { amounts: true } }
                        Text { Layout.preferredWidth: table.shareW; horizontalAlignment: Text.AlignRight
                               text: table.shareText(rowBox.p); font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: Theme.secondary
                               Sensitive { amounts: true } }
                        GainBlock { p: rowBox.p; Layout.preferredWidth: table.gainW }
                        RowButtons { p: rowBox.p; Layout.preferredWidth: table.editW }
                    }
                    // narrow: name with its subline on the left, value with gain and share on the right
                    ColumnLayout {
                        visible: !table.wide
                        anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
                        anchors.leftMargin: 14; anchors.rightMargin: 14; anchors.topMargin: 5
                        spacing: 0
                        RowLayout {
                            id: narrowRow
                            Layout.fillWidth: true; Layout.minimumHeight: 44
                            spacing: 10
                            ColumnLayout { spacing: 2; Layout.fillWidth: true
                                Text { text: rowBox.p.displayName; textFormat: Text.PlainText; Layout.fillWidth: true; elide: Text.ElideRight; wrapMode: rowBox.open ? Text.WordWrap : Text.NoWrap
                                       maximumLineCount: rowBox.open ? 3 : 1; font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text
                                       Sensitive { labels: !rowBox.p.isOwnItem } }
                                // quantity and price are figures: protected like amounts
                                Text { text: table.narrowSubline(rowBox.p); Layout.fillWidth: true; elide: Text.ElideRight
                                       font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: Theme.secondary
                                       Sensitive { amounts: true } }
                                Text { readonly property var label: table.depotLabel ? table.depotLabel(rowBox.p) : null
                                       visible: !!label; text: label || ""; Layout.fillWidth: true; elide: Text.ElideRight
                                       font.pixelSize: Theme.caption; color: Theme.tertiary; Sensitive {} } }
                            ColumnLayout { spacing: 2
                                Text { Layout.alignment: Qt.AlignRight; text: table.valueText(rowBox.p)
                                       font.pixelSize: Theme.body; font.weight: Font.Medium; font.features: { "tnum": 1 }; color: Theme.text; Sensitive { amounts: true } }
                                Text { Layout.alignment: Qt.AlignRight; text: [table.gainText(rowBox.p), table.shareText(rowBox.p)].filter(x => x).join(" · ")
                                       font.pixelSize: Theme.caption; font.features: { "tnum": 1 }
                                       color: rowBox.p.gain ? DepotMetrics.tint(rowBox.p.gain.cents) : Theme.secondary; Sensitive { amounts: true } } }
                            // a narrow window keeps the pencil at hand
                            RowButtons { p: rowBox.p }
                            Item {
                                id: detailsButton
                                implicitWidth: 18; implicitHeight: 18
                                SFIcon { anchors.centerIn: parent; symbol: "chevron.right"; size: 10; color: Theme.secondary; rotation: rowBox.open ? 90 : 0
                                         Behavior on rotation { NumberAnimation { duration: 200 } } }
                                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: table.toggle(rowBox.p.id) }
                                // a Button on the Mac: Tab reaches it, Space / Return opens the details
                                activeFocusOnTab: true
                                Keys.onSpacePressed: table.toggle(rowBox.p.id)
                                Keys.onReturnPressed: table.toggle(rowBox.p.id)
                                Keys.onEnterPressed: table.toggle(rowBox.p.id)
                                Rectangle { visible: detailsButton.activeFocus; anchors.fill: parent; anchors.margins: -2
                                            radius: 4; color: "transparent"; border.width: 2; border.color: Theme.accent }
                                Accessible.role: Accessible.Button
                                Accessible.name: rowBox.open ? qsTr("Details ausblenden") : qsTr("Details anzeigen")
                                Accessible.onPressAction: table.toggle(rowBox.p.id)
                            }
                        }
                        // everything the wide table shows in columns, as labelled lines
                        ColumnLayout {
                            id: details
                            visible: rowBox.open
                            Layout.fillWidth: true; Layout.topMargin: 6; Layout.bottomMargin: 8; Layout.leftMargin: 2
                            spacing: 6
                            DetailLine { title: qsTr("Menge / Einheit"); value: table.quantityText(rowBox.p) + " " + table.unitText(rowBox.p) }
                            DetailLine { visible: !rowBox.p.hidesPrice; title: qsTr("Kurs"); value: table.priceText(rowBox.p) }
                            DetailLine { title: qsTr("Marktwert"); value: Fmt.money(rowBox.p.holding.value_cents, table.currency) }
                            DetailLine {
                                readonly property string tradeCurrency: DepotMetrics.nativeCurrency(rowBox.p.holding, table.currency) || ""
                                visible: tradeCurrency !== "" && DepotMetrics.has(rowBox.p.holding.native_value_cents)
                                title: qsTr("Marktwert (Handelswährung)")
                                value: visible ? Fmt.money(rowBox.p.holding.native_value_cents, tradeCurrency) : ""
                            }
                            DetailLine { title: qsTr("Anteil"); value: table.shareText(rowBox.p) }
                            DetailLine { visible: !!rowBox.p.gain; title: qsTr("GuV"); value: table.gainText(rowBox.p)
                                         tint: rowBox.p.gain ? DepotMetrics.tint(rowBox.p.gain.cents) : Theme.text }
                            DetailLine { visible: rowBox.p.identifier !== ""; title: qsTr("ISIN / WKN"); value: rowBox.p.identifier }
                        }
                    }
                }
                Rectangle { visible: rowBox.index < table.positions.length - 1; Layout.fillWidth: true; Layout.leftMargin: 14; implicitHeight: 1
                            color: Theme.withAlpha(Theme.border, 0.7) }
            }
        }
        Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: Theme.border }

        // MARK: footer – totals under their columns
        ColumnLayout {
            id: footer
            readonly property real value: table.positions.reduce((t, p) => t + p.holding.value_cents, 0)
            readonly property real share: table.positions.reduce((t, p) => t + (p.share || 0), 0)
            // from the rows' own gains: merged positions and Sachwerte carry theirs
            readonly property var gain: DepotMetrics.unrealized(table.positions.map(p => p.gain))
            Layout.fillWidth: true; Layout.leftMargin: 14; Layout.rightMargin: 14; Layout.topMargin: 8; Layout.bottomMargin: 8
            spacing: 4
            RowLayout {
                Layout.fillWidth: true
                spacing: 10
                Text { Layout.fillWidth: true
                       text: table.positions.length === table.totalCount ? qsTr("%1 Positionen").arg(table.totalCount)
                                                                         : qsTr("%1 von %2 Positionen").arg(table.positions.length).arg(table.totalCount)
                       font.pixelSize: Theme.callout; font.weight: Font.DemiBold; color: Theme.secondary }
                Text { Layout.preferredWidth: table.wide ? table.valueW : -1; horizontalAlignment: Text.AlignRight
                       text: Fmt.money(footer.value, table.currency); font.pixelSize: Theme.callout; font.weight: Font.DemiBold; font.features: { "tnum": 1 }; color: Theme.text
                       Sensitive { amounts: true } }
                Text { visible: table.wide; Layout.preferredWidth: table.shareW; horizontalAlignment: Text.AlignRight
                       text: DepotMetrics.percent(footer.share * 100, false, 1); font.pixelSize: Theme.callout; font.weight: Font.DemiBold; font.features: { "tnum": 1 }; color: Theme.secondary
                       Sensitive { amounts: true } }
                // amount and percentage like the rows above; the percentage is the sum over the sum of costs
                ColumnLayout {
                    visible: table.wide
                    Layout.preferredWidth: table.gainW
                    spacing: 1
                    Text { Layout.alignment: Qt.AlignRight; text: footer.gain ? DepotMetrics.signedMoney(footer.gain.amountCents, table.currency) : "–"
                           font.pixelSize: Theme.callout; font.weight: Font.DemiBold; font.features: { "tnum": 1 }
                           color: footer.gain ? DepotMetrics.tint(footer.gain.amountCents) : Theme.tertiary; Sensitive { amounts: true } }
                    Text { visible: !!footer.gain && footer.gain.percent !== null; Layout.alignment: Qt.AlignRight
                           text: footer.gain && footer.gain.percent !== null ? DepotMetrics.percent(footer.gain.percent, true) : ""
                           font.pixelSize: Theme.callout; font.weight: Font.DemiBold; font.features: { "tnum": 1 }
                           color: footer.gain ? DepotMetrics.tint(footer.gain.amountCents) : Theme.tertiary; Sensitive { amounts: true } }
                }
                Item { visible: table.wide; Layout.preferredWidth: table.editW }
            }
            Text {
                visible: table.notesCalculatedGain && table.anyCalculatedGain
                Layout.fillWidth: true; wrapMode: Text.WordWrap
                text: qsTr("GuV teils aus dem Einstandskurs berechnet; Positionen ohne Einstandskurs sind nicht enthalten.")
                font.pixelSize: Theme.caption2; color: Theme.tertiary
            }
        }
    }

    // MARK: context menu – classification, the host's entries, copying
    Component { id: actionEntry; AppMenuItem { property var run; onTriggered: run() } }
    Component { id: actionDivider; AppMenuSeparator {} }
    AppMenu {
        id: rowMenu
        property var position: null
        property var entries: []
        function rebuild() {
            for (const o of entries) removeItem(o)
            const built = []
            const actions = table.rowActions && position ? table.rowActions(position) : []
            for (const a of actions) {
                if (a.separator) built.push(actionDivider.createObject(null))
                else built.push(actionEntry.createObject(null, { text: a.title, run: a.action }))
            }
            built.forEach((o, i) => insertItem(i + 2, o))
            entries = built
        }
        AppMenuItem { text: qsTr("Klassifizierung bearbeiten"); icon.name: SF.icon("pencil")
                      visible: !!rowMenu.position && !!rowMenu.position.asset.classification.security_key; height: visible ? implicitHeight : 0
                      onTriggered: table.classify(rowMenu.position) }
        AppMenuItem { text: qsTr("Wertverlauf anzeigen"); icon.name: SF.icon("chart.xyaxis.line")
                      visible: table.hasHistory(rowMenu.position); height: visible ? implicitHeight : 0
                      onTriggered: table.historyRequested(rowMenu.position) }
        AppMenuItem { text: qsTr("Position kopieren"); icon.name: SF.icon("doc.on.doc")
                      onTriggered: AppModel.copyToClipboard(DepotMetrics.clipboardLine(rowMenu.position.holding, table.currency)) }
        AppMenuItem { text: qsTr("Alle Positionen kopieren"); icon.name: SF.icon("doc.on.doc"); onTriggered: table.copyAll() }
    }

    // MARK: parts
    component HeadLabel: Text {
        property string title
        text: title; horizontalAlignment: Text.AlignRight; elide: Text.ElideRight
        font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.secondary
    }
    component SortHeader: Item {
        id: sh
        property string title
        property string key
        property bool trailing: false
        readonly property bool active: table.sort.key === key
        implicitHeight: shRow.implicitHeight; implicitWidth: shRow.implicitWidth
        RowLayout {
            id: shRow
            anchors.right: sh.trailing ? parent.right : undefined
            anchors.left: sh.trailing ? undefined : parent.left
            spacing: 3
            Text { text: sh.title; font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: sh.active ? Theme.text : Theme.secondary }
            SFIcon { symbol: "chevron.down"; rotation: table.sort.ascending ? 180 : 0; size: 8; color: Theme.text; opacity: sh.active ? 1 : 0 }
        }
        MouseArea { id: shArea; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
                    onClicked: table.sortRequested(DepotMetrics.toggledSort(table.sort, sh.key)) }
        ToolTip.visible: shArea.containsMouse; ToolTip.delay: 600; ToolTip.text: qsTr("Nach dieser Spalte sortieren")
        // a Button on the Mac: Tab reaches it, Space / Return sorts
        activeFocusOnTab: true
        Keys.onSpacePressed: table.sortRequested(DepotMetrics.toggledSort(table.sort, sh.key))
        Keys.onReturnPressed: table.sortRequested(DepotMetrics.toggledSort(table.sort, sh.key))
        Keys.onEnterPressed: table.sortRequested(DepotMetrics.toggledSort(table.sort, sh.key))
        Rectangle { visible: sh.activeFocus; anchors.fill: shRow; anchors.margins: -2
                    radius: 4; color: "transparent"; border.width: 2; border.color: Theme.accent }
        Accessible.onPressAction: table.sortRequested(DepotMetrics.toggledSort(table.sort, sh.key))
        Accessible.role: Accessible.ColumnHeader
        Accessible.name: title
        Accessible.description: active ? (table.sort.ascending ? qsTr("aufsteigend sortiert") : qsTr("absteigend sortiert")) : ""
    }
    component NameBlock: ColumnLayout {
        id: nb
        property var p
        spacing: 2
        Text { text: nb.p.displayName; textFormat: Text.PlainText; Layout.fillWidth: true; elide: Text.ElideRight
               font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text
               Sensitive { labels: !nb.p.isOwnItem } }
        RowLayout {
            Layout.fillWidth: true
            spacing: 3
            Text { visible: nb.p.identifier !== ""; text: nb.p.identifier + " ·"; font.pixelSize: Theme.callout; color: Theme.secondary; Sensitive {} }
            Text { text: AssetClassification.compactTitle(nb.p.asset.classification); font.pixelSize: Theme.callout; color: Theme.secondary
                   Layout.fillWidth: !depotText.visible; elide: Text.ElideRight
                   Sensitive { labels: true }
                   HoverHandler { id: classHover }
                   ToolTip.visible: classHover.hovered; ToolTip.delay: 600; ToolTip.text: AssetClassification.sourceDescription(nb.p.asset.classification) }
            Text { id: depotText; readonly property var label: table.depotLabel ? table.depotLabel(nb.p) : null
                   visible: !!label; text: "· " + (label || ""); Layout.fillWidth: true; elide: Text.ElideRight
                   font.pixelSize: Theme.callout; color: Theme.secondary; Sensitive {} }
        }
    }
    component GainBlock: ColumnLayout {
        id: gb
        property var p
        spacing: 1
        Text { visible: !!gb.p.gain; Layout.alignment: Qt.AlignRight; text: gb.p.gain ? DepotMetrics.signedMoney(gb.p.gain.cents, table.currency) : ""
               font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: gb.p.gain ? DepotMetrics.tint(gb.p.gain.cents) : Theme.text
               Sensitive { amounts: true } }
        Text { visible: !!gb.p.gain && gb.p.gain.percent !== null; Layout.alignment: Qt.AlignRight
               text: gb.p.gain && gb.p.gain.percent !== null ? DepotMetrics.percent(gb.p.gain.percent, true) : ""
               font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: gb.p.gain ? DepotMetrics.tint(gb.p.gain.cents) : Theme.text
               Sensitive { amounts: true } }
        Text { visible: !gb.p.gain; Layout.alignment: Qt.AlignRight; text: "–"; font.pixelSize: Theme.callout; color: Theme.tertiary }
        HoverHandler { id: gainHover }
        ToolTip.visible: gainHover.hovered; ToolTip.delay: 600
        ToolTip.text: !gb.p.gain ? qsTr("Kein Einstandskurs geliefert") : gb.p.gain.reported ? qsTr("Unrealisierte GuV laut Bank") : qsTr("Aus dem Einstandskurs berechnet")
    }
    /// The chart button (only with a value history) beside the pencil.
    component RowButtons: RowLayout {
        id: rb
        property var p
        spacing: 0
        Item {
            visible: table.hasHistory(rb.p)
            implicitWidth: 22; implicitHeight: 22
            IconButton {
                anchors.centerIn: parent
                symbol: "chart.xyaxis.line"; symbolSize: 12
                onClicked: table.historyRequested(rb.p)
                ToolTip.visible: hovered; ToolTip.delay: 600; ToolTip.text: qsTr("Wertverlauf anzeigen")
                Accessible.name: qsTr("Wertverlauf anzeigen")
            }
        }
        EditButton { p: rb.p }
    }
    /// Edits asset class and instrument; only where the position has a stable classification key.
    component EditButton: Item {
        id: eb
        property var p
        implicitWidth: 22; implicitHeight: 22
        IconButton {
            anchors.centerIn: parent
            visible: !!eb.p.asset.classification.security_key
            symbol: "pencil"; symbolSize: 12
            onClicked: table.classify(eb.p)
            ToolTip.visible: hovered; ToolTip.delay: 600; ToolTip.text: qsTr("Klassifizierung bearbeiten")
            Accessible.name: qsTr("Klassifizierung bearbeiten")
        }
    }
    component DetailLine: RowLayout {
        id: dl
        property string title
        property string value
        property color tint: Theme.text
        Layout.fillWidth: true
        Text { text: dl.title; font.pixelSize: Theme.callout; color: Theme.secondary }
        Item { Layout.fillWidth: true; Layout.minimumWidth: 12 }
        Text { text: dl.value; font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: dl.tint; horizontalAlignment: Text.AlignRight
               Sensitive { amounts: true } }
    }
}
