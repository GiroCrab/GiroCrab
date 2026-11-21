import Foundation
import GiroCrabCore

/// `girocrab://transfer?name=…&iban=…&bic=…&amount=…&purpose=…`: another app or a web page
/// asks GiroCrab to open a transfer form with these fields (docs/payment-link.md). The link
/// only fills the form; nothing is ever sent without the user's approval in the sheet.
enum PaymentLink {
    static let scheme = "girocrab"
    static let transferHost = "transfer"

    enum Failure: Error, Equatable {
        /// Not a `girocrab://transfer` link.
        case unsupported
        /// SEPA transfers are in euro only.
        case currency(String)
        /// Without an IBAN there is no recipient to prefill.
        case missingIBAN

        var message: String {
            switch self {
            case .unsupported:
                return String(localized: "GiroCrab kann mit diesem Link nichts anfangen.")
            case .currency(let code):
                return String(localized: "Der Link verlangt eine Zahlung in \(code). Überweisungen sind nur in Euro möglich.")
            case .missingIBAN:
                return String(localized: "Der Link enthält keine IBAN.")
            }
        }
    }

    /// Whether `url` is meant for this parser (other `girocrab://` links may follow later).
    static func handles(_ url: URL) -> Bool {
        url.scheme?.lowercased() == scheme
    }

    /// The payment of a transfer link. Texts are trimmed and single-line, the name capped at
    /// 70 and the purpose at 140 characters (SEPA); the amount accepts `12.50` and `12,50`.
    static func payment(from url: URL) -> Result<ParsedPayment, Failure> {
        guard handles(url), url.host?.lowercased() == transferHost,
              let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems
        else { return .failure(.unsupported) }
        func value(_ key: String) -> String? {
            let raw = items.first { $0.name.lowercased() == key }?.value ?? ""
            let line = raw.components(separatedBy: .newlines).joined(separator: " ")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            return line.isEmpty ? nil : line
        }
        if let currency = value("currency"), currency.uppercased() != "EUR" {
            return .failure(.currency(currency.uppercased()))
        }
        guard let iban = value("iban")?.filter({ !$0.isWhitespace }).uppercased(), !iban.isEmpty else {
            return .failure(.missingIBAN)
        }
        return .success(ParsedPayment(
            name: value("name").map { String($0.prefix(70)) },
            iban: iban,
            bic: value("bic")?.filter { !$0.isWhitespace }.uppercased(),
            amountText: value("amount").flatMap { parseAmountLoose(text: $0) }.flatMap { $0 > 0 ? Money.plain($0) : nil },
            purpose: value("purpose").map { String($0.prefix(140)) }
        ))
    }
}
