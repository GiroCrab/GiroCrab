pragma Singleton
import QtQuick

// The list filters added after 0.8.0, kept apart from AppModel.qml:
// - AppModel.swift › amountFilter / interactiveAmountFilter and AppModelTypes.swift › AmountFilter
//   (#357): only bookings whose absolute amount lies in the range; filtered in the core, so count,
//   pages, sums and exports follow. Kept while the app runs, like the other list filters.
// - AppModel.swift › hasActiveListFilters / resetListFilters („Filter zurücksetzen“, #840).
// - AppModel+Selection.swift › rememberPeriod / storedPeriod (#840): the period of the last launch.
// The header pill and its popover: StreamHeaderCard.qml / AmountFilterPopover.qml.
QtObject {
    id: lf

    /// AmountFilter.minCents / maxCents: absolute cents (the sign is the direction filter's business); null = open.
    property var amountMin: null
    property var amountMax: null
    /// AmountFilter.isActive
    readonly property bool amountActive: amountMin !== null || amountMax !== null
    /// Set while `reset` changes several filters: their own reloads are skipped (suspendsFilterReloads).
    property bool suspendsReloads: false

    /// interactiveAmountFilter = …: a range given the wrong way round is swapped (AmountFilter.normalized).
    function setAmount(lo, hi) {
        if (lo !== null && hi !== null && lo > hi) { const t = lo; lo = hi; hi = t }
        if (lo === amountMin && hi === amountMax) return
        amountMin = lo
        amountMax = hi
        if (suspendsReloads) return
        AppModel.listLimit = AppModel.defaultListLimit
        AppModel.reloadAfterFilterChange()
    }
    function clearAmount() { setAmount(null, null) }

    /// AmountFilter.title: label of the header pill, „Betrag“ or the range. Contains amounts: show it
    /// with `Sensitive { amounts: true }` while a range is active.
    readonly property string amountTitle: {
        const lo = amountMin, hi = amountMax
        if (lo === null && hi === null) return qsTr("Betrag")
        if (lo !== null && hi !== null) return lo === hi ? qsTr("Betrag: %1").arg(Fmt.money(lo))
                                                         : qsTr("Betrag: %1 – %2").arg(Fmt.money(lo)).arg(Fmt.money(hi))
        return lo !== null ? qsTr("Betrag: ≥ %1").arg(Fmt.money(lo)) : qsTr("Betrag: ≤ %1").arg(Fmt.money(hi))
    }

    /// AppModel+Loading.swift › currentFilter: `amount_min_cents` / `amount_max_cents`.
    function applyTo(f) {
        f.amount_min_cents = amountMin
        f.amount_max_cents = amountMax
        return f
    }

    /// hasActiveListFilters: any filter of the header row (status, type, tags, accounts, amount, search) narrows the list.
    readonly property bool hasActive: AppModel.statusFilter !== "all" || AppModel.directionFilter !== "All"
        || TagFilter.isActive || amountActive || AppModel.searchText !== "" || AppModel.categoryAccountIDs.length > 0

    /// resetListFilters: clears every list filter with a single reload; the period stays.
    function reset() {
        suspendsReloads = true
        AppModel.statusFilter = "all"
        AppModel.directionFilter = "All"
        TagFilter.tagIDs = []
        TagFilter.untagged = false
        amountMin = null
        amountMax = null
        AppModel.categoryAccountFilter = ({})
        AppModel.searchText = ""
        AppModel.searchTimer.stop()
        suspendsReloads = false
        AppModel.listLimit = AppModel.defaultListLimit
        AppModel.reloadAfterFilterChange()
    }

    // MARK: period across launches (#840)
    /// QSettings `period/last` (UserDefaults `period.last`): Period.lastChoiceKey – `storageKey`
    /// plus "custom", whose dates are saved separately.
    function rememberPeriod() {
        const p = AppModel.period
        AppModel.defaults.setValue("period/last", p === "custom" ? "custom" : AppModel.periodStorageKey(p) || "all")
    }
    /// storedPeriod: the period last chosen; null = none (or no longer valid).
    function storedPeriod() {
        const key = String(AppModel.defaults.value("period/last", ""))
        if (key === "") return null
        return key === "custom" ? "custom" : AppModel.periodFromStorageKey(key)
    }
}
