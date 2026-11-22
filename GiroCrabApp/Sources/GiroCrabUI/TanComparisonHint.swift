import SwiftUI

/// What the user compares on the TAN device before approving a single transfer (#949): the amount and the
/// end of the recipient IBAN. The bank builds the TAN request from the data it actually received, so this
/// comparison is the protection against manipulated order data. Shown by `TwoFactorSheet`, only for the
/// order's own TAN and with the values frozen when it was sent (`AppModel.tanComparison`).
struct TanComparison: Equatable {
    /// Recipient IBAN in groups of four.
    let iban: String
    /// The last four characters of the IBAN (what the user compares; may end in letters).
    let endCharacters: String
    let amount: String

    static let endLength = 4

    init?(iban: String, amountCents: Int64) {
        let grouped = TransferSheetView.groupedIBAN(iban)
        let compact = grouped.replacingOccurrences(of: " ", with: "")
        guard compact.count >= Self.endLength else { return nil }
        self.iban = grouped
        self.endCharacters = String(compact.suffix(Self.endLength))
        self.amount = Money.format(amountCents)
    }

    /// The grouped IBAN without its last four characters (spaces do not count).
    var ibanPrefix: String {
        var remaining = Self.endLength
        var chars = Array(iban)
        while remaining > 0, let last = chars.popLast() {
            if last != " " { remaining -= 1 }
        }
        while chars.last == " " { chars.removeLast() }
        return String(chars)
    }
}

/// The comparison card above the TAN field (part of `TwoFactorSheet`): amount and IBAN end large, plus one
/// sentence of instruction.
struct TanComparisonHint: View {
    let comparison: TanComparison

    /// Localization key of the instruction ("Zeichen": an IBAN may end in letters).
    static let instruction = "Bitte gleichen Sie Betrag und die letzten 4 Zeichen der Empfänger-IBAN vor der Freigabe auf Ihrem TAN-Gerät / Smartphone ab."

    var body: some View {
        VStack(spacing: 8) {
            VStack(spacing: 2) {
                Text("Betrag").appFont(.caption).foregroundStyle(.secondary)
                Text(comparison.amount)
                    .appFont(.title, weight: .bold, monospacedDigit: true)
                    .fixedSize()
                    .sensitive(.amounts)
            }
            VStack(spacing: 2) {
                Text("Empfänger-IBAN").appFont(.caption).foregroundStyle(.secondary)
                Text(comparison.ibanPrefix)
                    .appFont(.callout, design: .monospaced)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                    .sensitive()
                Text(comparison.endCharacters)
                    .appFont(.largeTitle, design: .monospaced)
                    .foregroundStyle(Theme.accent)
                    .fixedSize()
                    .sensitive()
            }
            Text(LocalizedStringKey(Self.instruction))
                .appFont(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(12)
        .frame(maxWidth: .infinity)
        .background(RoundedRectangle(cornerRadius: 10).fill(Theme.card))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.border))
        .accessibilityElement(children: .combine)
    }
}
