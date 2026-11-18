import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// DepotDetailView.swift (#564): a securities depot as a view of its own – value, four depot
// figures, composition, positions or activities. No cash flow, budgets, booking scopes or booking
// filters; the search (DepotState.query) and the sort are the view's own. The tab (Strg+1 / Strg+2)
// is DepotState.depotTab, switched in the toolbar.
AssetAccountFrame {
    id: dv
    property var account: null
    readonly property string currency: account && account.currency ? account.currency : "EUR"

    /// DepotLoadState: "loading" | "failed" | "loaded"; data of this depot only.
    property string loadState: "loading"
    property string failure: ""
    property var assets: []
    property var transactions: []
    /// The depot's value history (all of it; the period cuts it for the chart, #578).
    property var valuePoints: []
    /// Held positions with a value history by position id (DepotMetrics.makePositions), worked out once per load.
    property var heldHistory: ({})
    /// Positions no longer held that have a history, the latest sale first.
    property var soldPositions: []
    /// PositionHistoryRequest for PositionHistorySheet (null: closed).
    property var historyRequest: null
    property bool demoSheetShown: false
    property int dataAccount: -1
    readonly property bool loaded: loadState === "loaded" && account !== null && dataAccount === account.id
    readonly property var holdings: loaded ? assets.map(a => a.holding) : []

    property var sort: ({ key: "value", ascending: false })
    property string activityFilter: "all"
    property string compositionMode: "position"
    /// Positions behind the slice clicked in the composition card (null: all).
    property var compositionFilter: null

    readonly property real syncStamp: {
        const a = account
        const c = a ? AppModel.connections.find(x => x.id === a.connection_id) : null
        return c && c.last_sync_unix ? c.last_sync_unix : 0
    }
    /// Anything that can change what the depot shows reloads it: its value, a sync, a classification, the vault.
    readonly property string loadKey: account ? [account.id, account.balance_cents, account.balance_date, AssetClassification.version,
                                                 syncStamp, AppModel.vaultGeneration].join("-") : ""
    onLoadKeyChanged: Qt.callLater(reload)
    property int generation: 0
    function reload() {
        if (!account) return
        const id = account.id, gen = ++generation
        if (!(loadState === "loaded" && dataAccount === id)) loadState = "loading"
        Promise.all([AppModel.corePromise("classified_assets", { account_id: id }),
                     AppModel.corePromise("list_security_transactions", { account_id: id }),
                     AppModel.corePromise("depot_value_history", { account_ids: [id], days: DepotValueHistory.lookbackDays }),
                     AppModel.corePromise("depot_value_positions", { account_id: id })]).then(r => {
            if (gen !== generation) return
            // DepotData.init: held positions find their history by `holding_position_key`
            const held = {}
            for (const v of r[3]) if (v.held && !held[v.position]) held[v.position] = v
            const byId = {}
            for (const p of DepotMetrics.makePositions(r[0])) {
                const key = AppModel.core("holding_position_key", { isin: p.holding.isin, name: p.holding.name })
                if (key && held[key] && !byId[p.id]) byId[p.id] = held[key]
            }
            assets = r[0]; transactions = r[1]; valuePoints = r[2]; heldHistory = byId
            soldPositions = r[3].filter(v => !v.held).sort((a, b) => a.last_date < b.last_date ? 1 : a.last_date > b.last_date ? -1 : 0)
            dataAccount = id; loadState = "loaded"
            // (developer) `--demo-sheet position-history`: once, the first position that has a history
            if (AppModel.demoSheet === "position-history" && !demoSheetShown) {
                const first = DepotMetrics.makePositions(r[0]).find(p => byId[p.id])
                if (first) { demoSheetShown = true; openHistory(byId[first.id]) }
            }
        }, err => {
            if (gen !== generation) return
            failure = err && err.message ? err.message : String(err)
            loadState = "failed"
        })
    }

    /// PositionHistoryRequest(accountId:position:)
    function openHistory(v) {
        historyRequest = { accountId: account.id, key: v.position, name: v.name, currency: v.currency }
    }
    /// A sold position matches the search by name or key (ISIN).
    function matchesSold(v, query) {
        const q = query.trim().toLowerCase()
        return q === "" || v.name.toLowerCase().indexOf(q) >= 0 || v.position.toLowerCase().indexOf(q) >= 0
    }

    readonly property var value: DepotMetrics.resolveValue(account ? account.balance_cents : 0, holdings)
    readonly property var periodRange: DepotState.depotPeriodRange

    // MARK: header
    AssetHeaderCard {
        Layout.fillWidth: true
        account: dv.account
        title: dv.account ? dv.account.name : ""
        currency: dv.currency
        valueCents: dv.value.cents
        valueLabel: dv.value.source === "positionSum" ? qsTr("Summe der Positionen") : qsTr("Depotwert")
        periodForDepot: true
        valueHelp: qsTr("Depotwert laut Bank. Das Guthaben des Verrechnungskontos ist nicht enthalten.")
        valueNote: dv.value.differenceCents !== null
                   ? qsTr("Positionen: %1 (%2 Differenz)").arg(Fmt.money(dv.value.positionSumCents, dv.currency)).arg(DepotMetrics.signedMoney(dv.value.differenceCents, dv.currency)) : ""
        valueNoteHelp: qsTr("Der gemeldete Depotwert und die Summe der Positionen weichen voneinander ab, z. B. durch unterschiedliche Bewertungszeitpunkte.")
        // The depot's valuation history: the values the core holds (the bank's own reports at each
        // fetch, imported values), cut to the selected period. Before its first value the depot
        // has none: nothing is drawn there, and nothing is derived from bookings.
        chartContent: dv.loaded ? DepotValueHistory.chartContent(dv.valuePoints, dv.periodRange) : ({ kind: "loading" })
        chartFootnote: DepotValueHistory.footnote
        subtitle: [
            Text { visible: !!dv.account && dv.account.bank_name !== ""; text: dv.account ? dv.account.bank_name : ""
                   font.pixelSize: Theme.caption; color: Theme.secondary; elide: Text.ElideRight },
            // "Bewertung vom 14.11.2026 · Abruf vor 2 Stunden"; each part only when the data has it
            Text {
                readonly property var parts: [dv.account && dv.account.balance_date ? qsTr("Bewertung vom %1").arg(Fmt.medium(dv.account.balance_date)) : null,
                                              dv.syncStamp > 0 ? qsTr("Abruf %1").arg(Fmt.relative(dv.syncStamp)) : null].filter(x => x)
                text: parts.length ? parts.join(" · ") : qsTr("Noch kein Abruf")
                font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap; maximumLineCount: 2
                Layout.fillWidth: true
            },
            Text { visible: dv.loaded; text: qsTr("%1 Positionen").arg(dv.assets.length); font.pixelSize: Theme.caption; color: Theme.secondary }
        ]
    }

    // MARK: four figures
    DepotMetricGrid {
        visible: dv.loaded
        Layout.fillWidth: true
        wide: dv.wide
        tiles: dv.loaded ? DepotMetrics.securitiesTiles(dv.assets, dv.transactions, dv.currency, DepotState.depotPeriodTitle, dv.periodRange[0], dv.periodRange[1]) : []
    }

    // MARK: composition
    Text {
        visible: dv.loaded && DepotState.depotTab === "positions" && dv.assets.length > 0
        Layout.leftMargin: 4
        text: qsTr("Aktueller Bestand · unabhängig vom gewählten Zeitraum")
        font.pixelSize: Theme.caption; color: Theme.secondary
    }
    AssetCompositionCard {
        visible: dv.loaded && DepotState.depotTab === "positions" && dv.assets.length > 0
        Layout.fillWidth: true
        assets: dv.loaded ? dv.assets : []
        currency: dv.currency
        mode: dv.compositionMode
        filter: dv.compositionFilter
        expandedKey: "depotAnalysisExpanded"
        displayKey: "depotCompositionDisplay"
        onModeRequested: m => dv.compositionMode = m
        onFilterRequested: keys => dv.compositionFilter = keys
    }

    // MARK: content
    // ContentUnavailableView „Depot konnte nicht geladen werden“
    ColumnLayout {
        visible: dv.loadState === "failed"
        Layout.alignment: Qt.AlignHCenter; Layout.topMargin: 30
        spacing: 6
        SFIcon { Layout.alignment: Qt.AlignHCenter; symbol: "exclamationmark.triangle"; size: 32 }
        Text { Layout.alignment: Qt.AlignHCenter; text: qsTr("Depot konnte nicht geladen werden"); font.pixelSize: Theme.title2; font.weight: Font.Bold; color: Theme.text }
        Text { Layout.alignment: Qt.AlignHCenter; text: dv.failure; font.pixelSize: Theme.body; color: Theme.secondary }
        Button { Layout.alignment: Qt.AlignHCenter; text: qsTr("Erneut versuchen"); onClicked: dv.reload() }
    }
    BusyIndicator { visible: dv.loadState === "loading"; running: visible; Layout.alignment: Qt.AlignHCenter; Layout.topMargin: 30
                    implicitWidth: 20; implicitHeight: 20; padding: 0 }

    // positions
    Unavailable {
        visible: dv.loaded && DepotState.depotTab === "positions" && dv.assets.length === 0
        Layout.topMargin: 30
        symbol: "chart.line.uptrend.xyaxis"; title: qsTr("Keine Positionen")
        message: qsTr("Positionen kommen mit dem nächsten Abruf. Wertpapiergeschäfte stehen unter „Aktivitäten“, Geldbewegungen als Buchungen im Verrechnungskonto.")
    }
    readonly property var positions: loaded ? DepotMetrics.makePositions(assets) : []
    readonly property var shownPositions: DepotMetrics.sortedPositions(
        DepotMetrics.filteredByKeys(DepotMetrics.filteredByQuery(positions, DepotState.query), compositionFilter, positions), sort)
    Unavailable {
        visible: dv.loaded && DepotState.depotTab === "positions" && dv.assets.length > 0 && dv.shownPositions.length === 0
        symbol: "magnifyingglass"; title: qsTr("Keine Treffer")
        message: qsTr("Keine Position passt zu „%1“.").arg(AppModel.redactedSearch(DepotState.query))
    }
    DepotPositionsTable {
        visible: dv.loaded && DepotState.depotTab === "positions" && dv.shownPositions.length > 0
        Layout.fillWidth: true
        positions: dv.shownPositions
        totalCount: dv.positions.length
        sort: dv.sort
        wide: dv.wide
        currency: dv.currency
        onSortRequested: s => dv.sort = s
        onCopyAll: AppModel.copyToClipboard(dv.positions.map(p => DepotMetrics.clipboardLine(p.holding)).join("\n"))
        historyAvailable: p => !!dv.heldHistory[p.id]
        onHistoryRequested: p => { const v = dv.heldHistory[p.id]; if (v) dv.openHistory(v) }
    }
    readonly property var shownSold: loaded ? soldPositions.filter(v => matchesSold(v, DepotState.query)) : []
    SoldPositionsSection {
        visible: dv.loaded && DepotState.depotTab === "positions" && dv.shownSold.length > 0 && dv.compositionFilter === null
        Layout.fillWidth: true
        positions: dv.shownSold
        onOpen: v => dv.openHistory(v)
    }
    PositionHistorySheet {
        request: dv.historyRequest
        onDismissed: dv.historyRequest = null
    }

    // activities
    DepotActivitiesView {
        visible: dv.loaded && DepotState.depotTab === "activities"
        Layout.fillWidth: true
        all: dv.loaded ? dv.transactions : []
        shown: dv.loaded ? DepotMetrics.filteredActivities(dv.transactions, dv.account.id, dv.activityFilter, DepotState.query, dv.periodRange[0], dv.periodRange[1]) : []
        filter: dv.activityFilter
        query: DepotState.query
        periodTitle: DepotState.depotPeriodTitle
        wide: dv.wide
        onFilterRequested: f => dv.activityFilter = f
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
