import Foundation
import GiroCrabCore

/// Account kinds as the UI names them, and the choices that make sense when
/// changing one. Shared by the sidebar menu (macOS and iOS) and the database
/// import; hence outside the macOS-only import view.
extension AccountKind {
    static let importChoices: [AccountKind] = [.checking, .savings, .creditCard, .paymentService, .brokerage, .cash, .loan]

    var label: String {
        switch self {
        case .checking: return String(localized: "Girokonto")
        case .savings: return String(localized: "Tagesgeld / Sparen")
        case .creditCard: return String(localized: "Kreditkarte")
        case .paymentService: return String(localized: "Zahlungsdienst")
        case .brokerage: return String(localized: "Depot")
        case .cash: return String(localized: "Bargeld")
        case .loan: return String(localized: "Darlehen")
        }
    }
}
