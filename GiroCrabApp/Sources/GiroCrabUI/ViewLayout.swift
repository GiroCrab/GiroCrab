import SwiftUI

/// Persisted view-layout switches shared by the View menu, the toolbar and
/// the settings (GitHub #233, #234).
enum ViewLayout {
    /// Charts and analysis cards above the booking list.
    static let statisticsKey = "showStatistics"
    /// Right-hand inspector column.
    static let inspectorKey = "showInspector"
    /// Monthly budgets: the card above the list and „Monatsbudget festlegen …“.
    static let budgetsKey = "showBudgets"
    /// Header card reduced to title, balance and the filter row: no balance
    /// chart, KPIs or loan progress, more room for the bookings (GitHub #358).
    static let compactHeaderKey = "compactHeader"

    /// Budgets show until switched off (Ansicht › Budgets anzeigen).
    static func showsBudgets(_ defaults: UserDefaults = .standard) -> Bool {
        defaults.object(forKey: budgetsKey) as? Bool ?? true
    }

    /// Which cards go above the list where analyses show (overview, or an
    /// account with „Auswertungen“ opened): each follows its own switch, so
    /// the budgets stay when the analyses are off.
    static func cards(statistics: Bool, budgets: Bool) -> (cashflow: Bool, budgets: Bool, any: Bool) {
        (statistics, budgets, statistics || budgets)
    }

    /// The header shows in full until minimised (Ansicht › Kopfbereich minimieren).
    static func compactHeader(_ defaults: UserDefaults = .standard) -> Bool {
        defaults.bool(forKey: compactHeaderKey)
    }
}
