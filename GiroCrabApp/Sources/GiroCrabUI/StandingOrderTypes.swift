import Foundation
import GiroCrabCore

/// The standing order being created, changed or deleted (#496).
struct StandingDraft {
    var action: StandingOrderAction = .create
    /// `standing_orders.id` of the order to change or delete.
    var orderRowID: Int64?
    var accountID: Int64?
    var name = ""
    var iban = ""
    var bic = ""
    var amountText = ""
    var purpose = ""
    var firstDate = StandingDraft.tomorrow
    var hasEnd = false
    var lastDate = Calendar.current.date(byAdding: .year, value: 1, to: StandingDraft.tomorrow) ?? Date()
    /// `M` (months) or `W` (weeks).
    var unit = "M"
    var interval = 1
    /// Day of the month (31 = last day) or weekday (1 = Monday … 7).
    var execDay = 1
    var pin = ""

    static var tomorrow: Date { Calendar.current.date(byAdding: .day, value: 1, to: Date()) ?? Date() }

    var cents: Int64? {
        guard let c = try? parseAmountToCents(text: amountText), c > 0 else { return nil }
        return c
    }
    var ibanValid: Bool { validateIban(iban: iban) }

    /// What the editor still needs before it can go to the review.
    var isValid: Bool {
        let n = name.trimmingCharacters(in: .whitespaces)
        let lastOK = !hasEnd || lastDate >= firstDate
        return accountID != nil && !n.isEmpty && n.count <= 70 && ibanValid && cents != nil
            && purpose.count <= 140 && lastOK && (1...99).contains(interval)
    }

    /// An existing order as a draft (change or delete).
    init(order o: StandingOrderInfo, action: StandingOrderAction) {
        self.action = action
        orderRowID = o.id
        accountID = o.accountId
        name = o.recipientName
        iban = BatchRow.grouped(o.recipientIban)
        amountText = Money.plain(o.amountCents)
        purpose = o.purpose
        firstDate = DateText.date(o.firstDate) ?? Self.tomorrow
        if let last = o.lastDate, let d = DateText.date(last) {
            hasEnd = true
            lastDate = d
        }
        unit = o.timeUnit
        interval = Int(o.interval)
        execDay = Int(o.execDay)
    }

    init() {}

    func request(demo: Bool) -> StandingOrderRequest? {
        guard let accountID, let cents else { return nil }
        let bic = bic.trimmingCharacters(in: .whitespaces)
        return StandingOrderRequest(
            accountId: accountID, action: action, orderRowId: orderRowID,
            recipientName: name, recipientIban: iban, recipientBic: bic.isEmpty ? nil : bic,
            amountCents: cents, purpose: purpose, firstDate: DateText.iso(firstDate),
            lastDate: hasEnd ? DateText.iso(lastDate) : nil, timeUnit: unit,
            interval: UInt32(interval), execDay: UInt32(execDay))
    }

    /// "monatlich", "alle 2 Wochen" …, the same words as the planned list.
    var rhythmText: String {
        if unit == "W" {
            return interval <= 1 ? String(localized: "wöchentlich") : String(localized: "alle \(interval) Wochen")
        }
        switch interval {
        case ...1: return String(localized: "monatlich")
        case 3: return String(localized: "vierteljährlich")
        case 6: return String(localized: "halbjährlich")
        case 12: return String(localized: "jährlich")
        default: return String(localized: "alle \(interval) Monate")
        }
    }

    /// "am 15." / "am letzten Tag des Monats" / "montags".
    var execDayText: String {
        if unit == "W" { return Self.weekdayName(execDay) }
        return execDay >= 31 ? String(localized: "am letzten Tag des Monats") : String(localized: "am \(execDay).")
    }

    /// Weekday names for 1 = Monday … 7 = Sunday.
    static func weekdayName(_ day: Int) -> String {
        let symbols = Calendar.current.weekdaySymbols  // Sunday first
        return symbols[day % 7]
    }
}
