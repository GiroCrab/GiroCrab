import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// DepotDetailView.swift (#564): a securities depot as a view of its own – value, four depot
// figures and positions. No cash flow, budgets, booking scopes or booking filters; the search
// (DepotState.query) and the sort are the view's own. The tab (Strg+1) is DepotState.depotTab.
AssetAccountFrame {
    id: dv
    property var account: null
    readonly property string currency: account && account.currency ? account.currency : "EUR"

    /// DepotLoadState: "loading" | "failed" | "loaded"; data of this depot only.
    property string loadState: "loading"
    property string failure: ""
    property var assets: []
    property var transactions: []
    property int dataAccount: -1
    readonly property bool loaded: loadState === "loaded" && account !== null && dataAccount === account.id
    readonly property var holdings: loaded ? assets.map(a => a.holding) : []

    property var sort: ({ key: "value", ascending: false })

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
                     AppModel.corePromise("list_security_transactions", { account_id: id })]).then(r => {
            if (gen !== generation) return
            assets = r[0]; transactions = r[1]; dataAccount = id; loadState = "loaded"
        }, err => {
            if (gen !== generation) return
            failure = err && err.message ? err.message : String(err)
            loadState = "failed"
        })
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
        // The depot's valuation history: there is no reliable one yet, so the chart says so
        // (nothing is derived from bookings or activities).
        chartContent: ({ kind: "unavailable", text: qsTr("Bewertungsverlauf noch nicht verfügbar") })
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
    readonly property var shownPositions: DepotMetrics.sortedPositions(DepotMetrics.filteredByQuery(positions, DepotState.query), sort)
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
