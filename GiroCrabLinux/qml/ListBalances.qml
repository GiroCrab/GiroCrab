pragma Singleton
import QtQuick
import QtCore

// Running balance of the booking list (#577), kept apart from AppModel.qml:
// - ViewLayout.swift › runningBalanceKey („Saldo anzeigen“ in the Darstellung menu and the settings),
// - AppModel+Loading.swift › selectionHasBalances / listHasBalances / balanceCurrency / listFilter,
// - AppModelTypes.swift › ListBalances: the balances of the loaded pages, by booking and by day.
// The rows and day headers read `bookings` / `days` (BookingStreamView.qml).
QtObject {
    id: lb

    property Settings store: Settings {
        /// `@AppStorage(ViewLayout.runningBalanceKey)`, off by default.
        property bool showRunningBalance: false
    }
    readonly property bool shown: store.showRunningBalance
    /// Ansicht › Saldo anzeigen changed (also from the settings window): switching it on reloads a
    /// list that can show balances; switching it off only drops them (runningBalanceSettingChanged).
    function setShown(on) {
        if (store.showRunningBalance === on) return
        store.showRunningBalance = on
        if (on && selectionHasBalances) AppModel.reloadBookings()
        else clear()
    }

    /// selectionHasBalances: running balances fit only a selection of accounts (one account, a group,
    /// all accounts), not a category or „Neue Umsätze“, whose rows come from accounts the view does
    /// not name. The list and the CSV export share it.
    readonly property bool selectionHasBalances: {
        const s = AppModel.sidebarSelection
        return s === "all" || s.startsWith("account:") || s.startsWith("group:") || s.startsWith("bank:") || s.startsWith("userGroup:")
    }
    /// listHasBalances: the list shows running balances.
    readonly property bool listHasBalances: shown && selectionHasBalances

    /// balanceCurrency: one account's own currency, else EUR (the core sums only EUR accounts).
    readonly property string currency: AppModel.selectedAccount ? (AppModel.selectedAccount.currency || "EUR") : "EUR"

    /// ListBalances.bookings / days: booking id → text, day → text (formatted once, RunningBalance).
    property var bookings: ({})
    property var days: ({})

    /// listFilter: `currentFilter` for the list's own pages, with the running balances it shows.
    /// Other callers (print, counts, exports) use `currentFilter` and skip that work.
    function listFilter() {
        const f = AppModel.currentFilter()
        f.with_balances = listHasBalances
        return f
    }
    /// publish: a fresh first page replaces what was there.
    function reset(page) {
        bookings = ({})
        days = ({})
        if (page) merge(page)
    }
    function clear() { reset(null) }
    /// loadMoreBookings: a further page adds its balances.
    function merge(page) {
        const rows = page.running_balances || [], dayRows = page.day_balances || []
        if (!rows.length && !dayRows.length) return
        const b = Object.assign({}, bookings), d = Object.assign({}, days)
        for (const r of rows) b[r.booking_id] = Fmt.money(r.balance_cents, currency)
        for (const r of dayRows) d[r.date] = Fmt.money(r.balance_cents, currency)
        bookings = b
        days = d
    }
}
