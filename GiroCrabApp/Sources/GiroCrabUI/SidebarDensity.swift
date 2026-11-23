import SwiftUI

/// How the sidebar lays out its account rows (#884). Local presentation preference.
enum SidebarDensity: String, CaseIterable, Identifiable {
    /// Name, owner initials, new-bookings count and balance in one line.
    case spacious
    /// Name and count above, bank and balance below.
    case compact

    static let defaultsKey = "sidebarDensity"
    /// The one-line row is the default (#884); compact is the option.
    static let defaultValue: SidebarDensity = .spacious
    var id: String { rawValue }

    var title: String {
        switch self {
        case .spacious: String(localized: "Großzügig")
        case .compact: String(localized: "Kompakt")
        }
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
