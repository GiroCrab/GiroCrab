pragma Singleton
import QtQuick

// AppModel+Depot.swift and AssetAccountFrame.swift › AccountPresentation (#564): THE single place
// that decides how the selected account is presented. Main.qml (routing, toolbar, menu, search) and
// the command palette follow `presentation`. Kept out of AppModel.qml.
QtObject {
    id: ds
    /// AppModel.depotKind(of:): both depot kinds are `Brokerage` accounts (no name heuristics);
    /// Sachwerte are the ones the asset store knows. "securities" | "tangible" | null.
    function depotKind(accountID) {
        const a = AppModel.account(accountID)
        if (!a || a.kind !== "Brokerage") return null
        return AppModel.isTangible(accountID) ? "tangible" : "securities"
    }
    /// "bookings" | "securities"; Sachwerte keep the booking list with their positions for now.
    readonly property string presentation: {
        void AppModel.tangibleAccountIDs
        const a = AppModel.selectedAccount
        return a && depotKind(a.id) === "securities" ? "securities" : "bookings"
    }
    readonly property var selectedSecuritiesDepot: presentation === "securities" ? AppModel.selectedAccount : null

    /// AppModel.depotTab: the depot's inventory (Strg+1).
    property string depotTab: "positions"
    readonly property var depotTabs: [{ id: "positions", title: qsTr("Positionen") }]

    /// `.searchable` of AssetAccountFrame: the holdings views have their own search, never the booking search.
    /// One per view (DepotDetailView keeps `@State query`): cleared on every new account.
    property string query: ""
    property Connections selectionReset: Connections {
        target: AppModel
        function onSidebarSelectionChanged() { ds.query = "" }
    }
    readonly property string searchPrompt: qsTr("Wertpapier, ISIN oder WKN")

    /// The depot view has no salary cycle: "since last salary" is the whole period there.
    readonly property var depotPeriodRange: { void AppModel.customPeriodFrom; void AppModel.customPeriodTo
                                              return AppModel.period === "sinceSalary" ? [null, null] : AppModel.periodRange() }
    readonly property string depotPeriodTitle: AppModel.period === "sinceSalary" ? AppModel.periodTitleOf("all") : AppModel.periodTitle

    /// AccountPresentation.menuTitle(index:scope:): the Strg+1…3 entries.
    function menuTitle(index, scopeTitle) {
        if (presentation === "securities") return index < depotTabs.length ? depotTabs[index].title : "–"
        return scopeTitle
    }
    /// applyScopeShortcut(index:scope:): booking scopes or depot tabs.
    function applyScopeShortcut(index, scope) {
        if (presentation === "securities") { if (index < depotTabs.length) depotTab = depotTabs[index].id }
        else if (presentation === "bookings") AppModel.scope = scope
    }
}
