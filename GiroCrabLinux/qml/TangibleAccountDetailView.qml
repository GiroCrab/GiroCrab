import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtCore

// TangibleAccountDetailView.swift (#87, #564): a Sachwerte account as a view of its own – held items
// in the shared positions table, sold ones in their own card, a value chart of the account or of the
// selected item, and the actions to add, revalue and sell. Items, valuations and price sources come
// from the Sachwerte store (AppModel.assetItems); nothing here is a securities position or a booking.
AssetAccountFrame {
    id: tv
    property var account: null
    readonly property string currency: account && account.currency ? account.currency : "EUR"
    minContentHeight: 420
    onBackgroundTapped: AppModel.deselectAssetItem()

    readonly property var items: account ? AppModel.assetItems(account.id) : []
    readonly property var heldItems: items.filter(i => !i.sold)
    /// The selected item, if there is one: its value and history instead of the account's.
    readonly property var item: { void AppModel.selectedAssetItemID; return account ? AppModel.selectedAssetItem(account.id) : null }

    /// classified_assets of the account; a failed query is shown with a retry, not swallowed.
    property var classified: []
    property string classificationError: ""
    property bool classificationLoaded: false
    /// Yesterday's value of one piece per automatically priced item: the day change of the figure tiles.
    property var previousUnits: ({})
    property string compositionMode: "assetClass"
    property var compositionFilter: null
    property var sort: ({ key: "value", ascending: false })
    property var deleting: null
    /// @AppStorage("showSoldAssets")
    Settings { id: prefs; property bool showSoldAssets: false }

    readonly property string loadKey: account ? [account.id, account.balance_cents, account.balance_date, AssetClassification.version,
                                                 AppModel.vaultGeneration, items.length].join("-") : ""
    onLoadKeyChanged: Qt.callLater(loadClassification)
    property int generation: 0
    function loadClassification() {
        if (!account) return
        const id = account.id, gen = ++generation
        const y = new Date(); y.setDate(y.getDate() - 1)
        const yesterday = Fmt.iso(y)
        const priced = heldItems.filter(i => i.setup.price_source !== "manual").map(i => i.id)
        Promise.all([AppModel.corePromise("classified_assets", { account_id: id }),
                     // a missing price leaves the day change unknown, it does not fail the load
                     Promise.all(priced.map(item => AppModel.corePromise("asset_unit_value_on", { item_id: item, date: yesterday }).then(v => v, () => null)))])
            .then(r => {
                if (gen !== generation) return
                const units = {}
                priced.forEach((item, i) => { if (r[1][i] !== null && r[1][i] !== undefined) units[item] = r[1][i] })
                classified = r[0]; previousUnits = units; classificationError = ""; classificationLoaded = true
            }, err => {
                if (gen !== generation) return
                classificationError = err && err.message ? err.message : String(err)
                classificationLoaded = true
            })
    }
    Connections { target: tv; function onAccountChanged() { AppModel.selectAssetItem(null) } }

    /// What is held now: sold items are not part of the current inventory.
    readonly property var heldAssets: classified.filter(a => { const it = DepotMetrics.has(a.asset_item_id) ? AppModel.assetItem(a.asset_item_id) : null; return it ? !it.sold : true })

    // MARK: header
    AssetHeaderCard {
        Layout.fillWidth: true
        account: tv.account
        // "Sachwerte · Auto (Kombi)" while an item is selected, else "Sachwerte · Gesamt"
        title: AppModel.title + " · " + (tv.item ? tv.item.setup.name : qsTr("Gesamt"))
        currency: tv.currency
        valueCents: tv.item ? tv.item.holding.value_cents : (tv.account ? tv.account.balance_cents : 0)
        valueLabel: tv.item && tv.item.sold ? qsTr("Verkauft") : qsTr("Wert heute")
        chartContent: {
            if (tv.item) {
                const r = AppModel.periodRange()
                return { kind: "points", points: AppModel.selectedAssetHistory.filter(p => (r[0] === null || p.date >= r[0]) && (r[1] === null || p.date <= r[1])) }
            }
            return AppModel.isLoadingSelection ? { kind: "loading" } : { kind: "points", points: AppModel.history }
        }
        subtitle: [
            Text { readonly property string line: tv.account ? [tv.account.bank_name, tv.account.iban ? Fmt.iban(tv.account.iban) : ""].filter(s => s).join(" · ") : ""
                   visible: line !== ""; text: line; font.pixelSize: Theme.caption; color: Theme.secondary; Sensitive {} },
            Text { readonly property string note: tv.account ? (tv.account.note || "") : ""
                   visible: note !== ""; text: note; font.pixelSize: Theme.caption; color: Theme.secondary
                   wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight; Layout.fillWidth: true
                   Sensitive {} },
            // "Bewertungen vom 16.11.2026 bis … · Goldpreis vom 16.11.2026"
            Text {
                readonly property var parts: {
                    const priceDate = tv.heldItems.filter(i => i.setup.price_source !== "manual").map(i => i.value_date).filter(d => d).sort().pop()
                    const dates = tv.heldItems.map(i => i.value_date).filter(d => d).sort()
                    const first = dates[0], last = dates[dates.length - 1]
                    const nb = s => s.replace(/vom /g, "vom\u00A0").replace(/bis /g, "bis\u00A0")
                    const valuation = first && last && first !== last ? qsTr("Bewertungen vom %1 bis %2").arg(Fmt.medium(first)).arg(Fmt.medium(last))
                        : last ? qsTr("Bewertung vom %1").arg(Fmt.medium(last))
                        : tv.account && tv.account.balance_date ? qsTr("Bewertung vom %1").arg(Fmt.medium(tv.account.balance_date)) : null
                    return [valuation ? nb(valuation) : null,
                            priceDate ? qsTr("Goldpreis vom %1").arg(Fmt.medium(priceDate)).replace(/ /g, "\u00A0") : null].filter(x => x)
                }
                visible: parts.length > 0
                text: parts.join(" · ")
                font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap; maximumLineCount: 4
                Layout.fillWidth: true
            },
            Text { visible: tv.heldItems.length > 0; text: qsTr("%1 Positionen").arg(tv.heldItems.length); font.pixelSize: Theme.caption; color: Theme.secondary }
        ]
        actions: [
            BusyIndicator { visible: AppModel.assetRefreshing && tv.items.some(i => i.setup.price_source !== "manual"); running: visible; implicitWidth: 18; implicitHeight: 18 },
            IconButton { visible: !AppModel.assetRefreshing && tv.items.some(i => i.setup.price_source !== "manual")
                         text: qsTr("Goldpreis aktualisieren"); symbol: "arrow.clockwise"; symbolSize: 12; font.pixelSize: Theme.caption
                         onClicked: AppModel.refreshAssetPrices(true)
                         ToolTip.visible: hovered; ToolTip.delay: 600; ToolTip.text: qsTr("Goldpreis jetzt abrufen") },
            IconButton { text: qsTr("Sachwert hinzufügen"); symbol: "plus"; symbolSize: 12; font.pixelSize: Theme.caption
                         onClicked: AppModel.openAssetEditor(tv.account.id) }
        ]
    }

    // MARK: figures and composition
    DepotMetricGrid {
        visible: tv.heldItems.length > 0
        Layout.fillWidth: true
        wide: tv.wide
        tiles: DepotMetrics.tangibleTiles(tv.heldItems, tv.previousUnits, tv.currency)
    }
    AssetCompositionCard {
        visible: tv.classificationLoaded && tv.classificationError === "" && tv.heldAssets.some(a => a.holding.value_cents > 0)
        Layout.fillWidth: true
        assets: tv.heldAssets
        currency: tv.currency
        modes: ["position", "assetClass"]
        mode: tv.compositionMode
        filter: tv.compositionFilter
        expandedKey: "tangibleAnalysisExpanded"
        displayKey: "tangibleCompositionDisplay"
        note: tv.item ? qsTr("Zusammensetzung des gesamten Kontos, nicht nur des gewählten Objekts") : ""
        onModeRequested: m => tv.compositionMode = m
        onFilterRequested: keys => tv.compositionFilter = keys
    }

    // MARK: content
    Rectangle {
        visible: tv.classificationError !== ""
        Layout.fillWidth: true
        implicitHeight: failCol.implicitHeight + 24
        radius: 12; color: Theme.card; border.color: Theme.withAlpha(Theme.warning, 0.6)
        ColumnLayout {
            id: failCol
            x: 16; y: 12; width: parent.width - 32
            spacing: 6
            RowLayout { spacing: 6
                SFIcon { symbol: "exclamationmark.triangle"; size: 13; color: Theme.warning }
                Text { text: qsTr("Klassifizierung konnte nicht geladen werden"); font.pixelSize: Theme.subheadline + 1; font.weight: Font.DemiBold; color: Theme.warning } }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   text: qsTr("Klassifizierungen fehlen: die Zusammensetzung, das Klassifizieren im Kontextmenü und die Suche nach Klassifizierung sind unvollständig.") }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; maximumLineCount: 3; text: tv.classificationError; font.pixelSize: Theme.caption2; color: Theme.tertiary }
            IconButton { text: qsTr("Erneut versuchen"); textColor: Theme.accent; font.pixelSize: Theme.caption; onClicked: tv.loadClassification() }
        }
    }

    // ContentUnavailableView „Noch keine Sachwerte“
    ColumnLayout {
        visible: tv.items.length === 0
        Layout.alignment: Qt.AlignHCenter; Layout.topMargin: 40
        spacing: 6
        SFIcon { Layout.alignment: Qt.AlignHCenter; symbol: "shippingbox"; size: 32 }
        Text { Layout.alignment: Qt.AlignHCenter; text: qsTr("Noch keine Sachwerte"); font.pixelSize: Theme.title2; font.weight: Font.Bold; color: Theme.text }
        Text { Layout.alignment: Qt.AlignHCenter; text: qsTr("Fügen Sie z. B. Goldmünzen, Barren oder ein Auto hinzu."); font.pixelSize: Theme.body; color: Theme.secondary }
        Button { Layout.alignment: Qt.AlignHCenter; Layout.topMargin: 6; text: qsTr("Sachwert hinzufügen …"); onClicked: AppModel.openAssetEditor(tv.account.id) }
    }

    // TangibleAssetsList: the search filters by name and classification; shares stay relative to everything held
    readonly property var matching: items.filter(i => {
        const c = classified.find(a => a.asset_item_id === i.id)
        return DepotMetrics.matches(DepotState.query, [i.setup.name, c ? AssetClassification.compactTitle(c.classification) : ""])
    })
    readonly property var allPositions: DepotMetrics.tangiblePositions(items, classified)
    readonly property var heldPositions: DepotMetrics.sortedPositions(DepotMetrics.filteredByKeys(
        allPositions.filter(p => matching.some(i => i.id === p.asset.asset_item_id)), compositionFilter, allPositions), sort)
    readonly property var sold: matching.filter(i => i.sold).sort((a, b) => a.setup.name.localeCompare(b.setup.name))

    ColumnLayout {
        visible: tv.items.length > 0 && tv.matching.length === 0
        Layout.alignment: Qt.AlignHCenter; Layout.topMargin: 20
        spacing: 6
        SFIcon { Layout.alignment: Qt.AlignHCenter; symbol: "magnifyingglass"; size: 32 }
        Text { Layout.alignment: Qt.AlignHCenter; text: qsTr("Keine Treffer"); font.pixelSize: Theme.title2; font.weight: Font.Bold; color: Theme.text }
        Text { Layout.alignment: Qt.AlignHCenter; text: qsTr("Kein Sachwert passt zu „%1“.").arg(AppModel.redactedSearch(DepotState.query)); font.pixelSize: Theme.body; color: Theme.secondary }
    }
    DepotPositionsTable {
        visible: tv.heldPositions.length > 0
        Layout.fillWidth: true
        positions: tv.heldPositions
        totalCount: tv.allPositions.length
        sort: tv.sort
        wide: tv.wide
        currency: tv.currency
        notesCalculatedGain: false
        selectable: true
        selectedID: { const p = tv.allPositions.find(p => AppModel.selectedAssetItemID !== null && p.asset.asset_item_id === AppModel.selectedAssetItemID); return p ? p.id : "" }
        onSortRequested: s => tv.sort = s
        onCopyAll: AppModel.copyToClipboard(tv.allPositions.map(p => DepotMetrics.clipboardLine(p.holding)).join("\n"))
        onSelected: p => AppModel.selectAssetItem(p.asset.asset_item_id)
        onOpened: p => {
            const id = p.asset.asset_item_id
            // the second click toggled the selection off again: keep the item selected
            if (AppModel.selectedAssetItemID !== id) AppModel.selectAssetItem(id)
            AppModel.openAssetEditor(tv.account.id, id)
        }
        rowActions: p => tv.itemActions(tv.items.find(i => i.id === p.asset.asset_item_id))
    }
    /// The item's own actions, in the table's context menu.
    function itemActions(it) {
        if (!it) return []
        const out = [{ title: qsTr("Sachwert bearbeiten …"), action: () => AppModel.openAssetEditor(tv.account.id, it.id) }]
        // gold follows the price API; only manual items carry own values
        if (it.setup.price_source === "manual") out.push({ title: qsTr("Bewertungen bearbeiten …"), action: () => AppModel.openAssetAction("values", it.id) })
        out.push({ title: qsTr("Anzahl bearbeiten …"), action: () => AppModel.openAssetAction("quantities", it.id) })
        if (!it.sold) out.push({ title: qsTr("Sachwert verkaufen …"), action: () => AppModel.openAssetAction("sell", it.id) })
        out.push({ separator: true })
        out.push({ title: qsTr("Sachwert löschen …"), action: () => tv.deleting = it })
        return out
    }

    // „Ehemalige Positionen (n)“
    Rectangle {
        visible: tv.sold.length > 0
        Layout.fillWidth: true
        implicitHeight: soldCol.implicitHeight
        radius: 12; color: Theme.card; border.color: Theme.border
        clip: true
        ColumnLayout {
            id: soldCol
            width: parent.width
            spacing: 0
            Item {
                id: soldToggle
                Layout.fillWidth: true
                implicitHeight: soldHead.implicitHeight + 20
                function toggle() { prefs.showSoldAssets = !prefs.showSoldAssets }
                // a Button on the Mac: Tab reaches it, Space / Return folds it
                activeFocusOnTab: true
                Keys.onSpacePressed: toggle()
                Keys.onReturnPressed: toggle()
                Keys.onEnterPressed: toggle()
                Rectangle { visible: soldToggle.activeFocus; anchors.fill: parent; anchors.margins: 2
                            radius: 6; color: "transparent"; border.width: 2; border.color: Theme.accent }
                RowLayout {
                    id: soldHead
                    x: 16; y: 10; width: parent.width - 32
                    spacing: 6
                    SFIcon { symbol: "chevron.right"; size: 10; rotation: prefs.showSoldAssets ? 90 : 0; color: Theme.secondary
                             Behavior on rotation { NumberAnimation { duration: 200 } } }
                    Text { text: qsTr("Ehemalige Positionen (%1)").arg(tv.sold.length); font.pixelSize: Theme.subheadline + 1
                           font.weight: Font.DemiBold; color: Theme.secondary; Layout.fillWidth: true }
                }
                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: soldToggle.toggle() }
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("Ehemalige Positionen (%1)").arg(tv.sold.length)
                Accessible.description: prefs.showSoldAssets ? qsTr("Ausgeklappt") : qsTr("Eingeklappt")
                Accessible.onPressAction: toggle()
            }
            Rectangle { visible: prefs.showSoldAssets; Layout.fillWidth: true; implicitHeight: 1; color: Theme.border }
            Repeater {
                model: prefs.showSoldAssets ? tv.sold : []
                ColumnLayout {
                    required property var modelData
                    required property int index
                    Layout.fillWidth: true
                    spacing: 0
                    SoldRow { it: modelData; Layout.fillWidth: true }
                    Rectangle { visible: index < tv.sold.length - 1; Layout.fillWidth: true; Layout.leftMargin: 64; implicitHeight: 1
                                color: Theme.withAlpha(Theme.border, 0.7) }
                }
            }
        }
    }

    ConfirmDialog {
        /// the item asked about (`deleting` is reset when the dialog closes)
        property var target: null
        visible: tv.deleting !== null
        onAboutToShow: target = tv.deleting
        heading: tv.deleting ? qsTr("„%1“ löschen?").arg(tv.deleting.setup.name) : ""
        message: qsTr("Der Sachwert und sein Verlauf werden entfernt.")
        actionTitle: qsTr("Löschen")
        onClosed: tv.deleting = null
        onConfirmed: {
            const it = target
            if (it) try { AppModel.changeAssets("delete_asset_item", { item_id: it.id }) } catch (e) { AppModel.errorMessage = AppModel.parseCoreError(e.message).message }
        }
    }

    function detail(it) {
        if (it.sold && it.quantities.length)
            return qsTr("verkauft am %1").arg(Fmt.medium(it.quantities[it.quantities.length - 1].date))
        if (it.missing_price_date) return qsTr("Kurs für den %1 fehlt").arg(Fmt.medium(it.missing_price_date))
        const qty = NumberText.format(it.holding.quantity) + " × " + Fmt.money(it.holding.current_price_cents)
        if (!it.value_date) return qty + " · " + qsTr("noch kein Kurs")
        const source = it.setup.price_source === "manual" ? qsTr("Wert vom %1").arg(Fmt.medium(it.value_date)) : qsTr("Kurs vom %1").arg(Fmt.medium(it.value_date))
        return qty + " · " + source
    }

    /// PositionCard.swift › PositionRow of a sold item, laid out like a booking row.
    component SoldRow: Rectangle {
        id: row
        property var it
        readonly property var classification: { const c = tv.classified.find(a => a.asset_item_id === it.id); return c ? c.classification : null }
        readonly property bool selected: AppModel.selectedAssetItemID === it.id
        implicitHeight: Math.max(44, rowLayout.implicitHeight) + 10
        color: selected ? Theme.selection : "transparent"
        RowLayout {
            id: rowLayout
            anchors.left: parent.left; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
            anchors.leftMargin: 14; anchors.rightMargin: 14
            spacing: 12
            opacity: 0.6
            AssetSymbolTile { symbol: AssetSymbols.symbolOf(row.it) }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Text { text: row.it.setup.name; font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text; elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 3
                    SFIcon { visible: !!row.it.missing_price_date; symbol: "exclamationmark.triangle.fill"; size: 11; color: Theme.warning }
                    Text { text: (row.classification ? AssetClassification.compactTitle(row.classification) + " · " : "") + tv.detail(row.it)
                           font.pixelSize: Theme.callout; color: Theme.secondary; elide: Text.ElideRight; Layout.fillWidth: true; Sensitive { amounts: true } }
                }
            }
            Text {
                visible: row.it.holding.value_cents !== 0
                text: Fmt.money(row.it.holding.value_cents)
                font.pixelSize: Theme.body; font.weight: Font.Medium; font.features: { "tnum": 1 }; color: Theme.text
                Sensitive { amounts: true }
            }
        }
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            onClicked: m => {
                if (m.button === Qt.RightButton) { menu.popup(); return }
                AppModel.selectAssetItem(row.it.id)
            }
            onDoubleClicked: m => {
                if (m.button !== Qt.LeftButton) return
                if (AppModel.selectedAssetItemID !== row.it.id) AppModel.selectAssetItem(row.it.id)
                AppModel.openAssetEditor(tv.account.id, row.it.id)
            }
        }
        Accessible.role: Accessible.Button
        Accessible.name: AppModel.masking(row.it.setup.name)
        AppMenu {
            id: menu
            AppMenuItem { text: qsTr("Klassifizierung bearbeiten"); visible: row.classification !== null; height: visible ? implicitHeight : 0
                          onTriggered: AssetClassification.edit(row.it.setup.name, row.classification) }
            AppMenuItem { text: qsTr("Sachwert bearbeiten …"); onTriggered: AppModel.openAssetEditor(tv.account.id, row.it.id) }
            AppMenuItem { text: qsTr("Bewertungen bearbeiten …"); visible: row.it.setup.price_source === "manual"; height: visible ? implicitHeight : 0
                          onTriggered: AppModel.openAssetAction("values", row.it.id) }
            AppMenuItem { text: qsTr("Anzahl bearbeiten …"); onTriggered: AppModel.openAssetAction("quantities", row.it.id) }
            AppMenuSeparator {}
            AppMenuItem { text: qsTr("Sachwert löschen …"); onTriggered: tv.deleting = row.it }
        }
    }
}
