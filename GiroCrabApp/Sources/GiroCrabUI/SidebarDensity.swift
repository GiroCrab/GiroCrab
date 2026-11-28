import SwiftUI

/// How the sidebar lays out its account rows (#884). Local presentation preference.
enum SidebarDensity: String, CaseIterable, Identifiable {
    /// Larger signet and title, owner initials, the bank below the name.
    case spacious
    /// Smaller signet and title, one line per account; the bank only where it tells accounts apart.
    case compact

    static let defaultsKey = "sidebarDensity"
    /// The one-line row is the default (#884); compact is the option.
    static let defaultValue: SidebarDensity = .spacious
    var id: String { rawValue }

    var usesTighterAccountSpacing: Bool {
        #if os(macOS)
        self == .compact
        #else
        false
        #endif
    }

    /// Repayment bar of a loan and the archive date: a second line only the spacious layout has.
    var showsStatusLine: Bool { self == .spacious }

    /// The line under the name. Compact keeps it to one line: the bank alone stays out, while
    /// what tells accounts apart (namesake distinction, owner or IBAN hit while searching) stays.
    /// A namesake keeps the bank line: then the bank may be all that tells the two apart.
    func showsSubtitle(_ text: String, bank: String, hasNamesake: Bool = false) -> Bool {
        !text.isEmpty && !(self == .compact && text == bank && !hasNamesake)
    }

    var title: String {
        switch self {
        case .spacious: String(localized: "Großzügig")
        case .compact: String(localized: "Kompakt")
        }
    }

    /// Initials tell accounts apart only when there is more than one owner.
    static func hasSeveralOwners(_ owners: [String]) -> Bool {
        Set(owners.map { $0.trimmingCharacters(in: .whitespaces).lowercased() }.filter { !$0.isEmpty }).count > 1
    }

    /// Owner initials as shown next to the name: first and last name ("Erika Mustermann" → "EM").
    /// Skips titles and other abbreviations ("Dr."), words without a leading letter ("&") and
    /// reads "Mustermann, Erika" as "Erika Mustermann" (a comma between two single words only).
    static func initials(of owner: String) -> String {
        let parts = owner.split(separator: ",", maxSplits: 1).map { String($0) }
        let singleWords = parts.count == 2 && parts.allSatisfy {
            $0.split(whereSeparator: \.isWhitespace).count == 1
        }
        let ordered = singleWords ? parts[1] + " " + parts[0] : owner
        let words = ordered.split(whereSeparator: \.isWhitespace)
            .filter { $0.first?.isLetter == true && !$0.hasSuffix(".") }
        guard let first = words.first?.first else { return "" }
        guard words.count > 1, let last = words.last?.first else { return String(first).uppercased() }
        return (String(first) + String(last)).uppercased()
    }
}
