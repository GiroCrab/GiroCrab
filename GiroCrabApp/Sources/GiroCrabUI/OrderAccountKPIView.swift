import SwiftUI
import GiroCrabCore

/// Evidence figures for the selected period, never a bank balance or an income KPI.
struct OrderAccountKPIView: View {
    let summary: OrderAccountSummary?
    let currency: String
    let period: String

    var body: some View {
        HStack(spacing: 10) {
            KPI(title: "Bestellungen", subtitle: LocalizedStringKey(period),
                value: summary.map { String($0.orderCount) } ?? "–", tint: Theme.text)
            KPI(title: "Gutschriften", subtitle: LocalizedStringKey(summary.map {
                Money.format($0.creditValueCents, currency: currency)
            } ?? "–"), value: summary.map { String($0.creditCount) } ?? "–", tint: Theme.inflow, subtitleIsSensitive: true)
            KPI(title: "Bestellwert", subtitle: "Vor Gutschriften",
                value: summary.map { Money.format($0.orderValueCents, currency: currency) } ?? "–", tint: Theme.text)
        }
        .frame(maxWidth: .infinity)
        .fixedSize(horizontal: false, vertical: true)
    }
}
