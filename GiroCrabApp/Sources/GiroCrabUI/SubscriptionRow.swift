import SwiftUI
import GiroCrabCore

/// Icon of a subscription: the merchant signet when a booking is known, else a symbol tile.
struct SubscriptionIcon: View {
    @Environment(AppModel.self) private var model
    let item: SubscriptionItem
    var size: CGFloat = 36
    @Environment(\.textScale) private var textScale

    var body: some View {
        if let b = model.subscriptionBookings[item.id]?.first {
            MerchantSignet(merchant: b.merchant, categorySymbol: b.categorySymbol, size: size)
        } else {
            let side = size * textScale
            Image(systemName: item.categorySymbol.isEmpty ? "repeat" : item.categorySymbol)
                .font(.system(size: side * 0.45)) // icon: does not scale with the text
                .foregroundStyle(Theme.secondary)
                .frame(width: side, height: side)
                .background(Theme.card, in: RoundedRectangle(cornerRadius: side * 0.28, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: side * 0.28, style: .continuous).strokeBorder(Theme.border))
                .accessibilityHidden(true)
        }
    }
}

/// Colour for a subscription that needs attention (nil: none).
extension SubscriptionAttention {
    var color: Color? {
        switch self {
        case .none, .ended: nil
        case .deadlineSoon: Theme.warning
        case .cancelledRunning: Theme.inflow
        }
    }
}

/// Contract status of a subscription in one line: cancelled, deadline, end.
struct SubscriptionContractLine: View {
    let item: SubscriptionItem

    private func day(_ date: Date) -> String { DateText.medium(DateText.iso(date)) }

    var body: some View {
        let d = item.details
        if let s = item.status {
            if d.cancelled {
                if s.ended {
                    Label("Beendet am \(day(s.end))", systemImage: "xmark.circle")
                        .appFont(.caption).foregroundStyle(Theme.secondary)
                } else {
                    Label("Gekündigt · läuft bis \(day(s.end))", systemImage: "checkmark.circle.fill")
                        .appFont(.caption, weight: .semibold).foregroundStyle(Theme.inflow)
                }
            } else if s.expired {
                Label("Vertrag abgelaufen am \(day(s.end))", systemImage: "exclamationmark.triangle")
                    .appFont(.caption).foregroundStyle(Theme.warning)
            } else if let deadline = s.deadline {
                if s.deadlinePassed {
                    Text("Kündigungsfrist am \(day(deadline)) verstrichen · Ende \(day(s.end))")
                        .appFont(.caption).foregroundStyle(Theme.secondary)
                } else if let days = s.daysToDeadline, days <= 30 {
                    Label(days == 0 ? String(localized: "Kündigen nur noch heute · Ende \(day(s.end))")
                                    : String(localized: "Kündigen bis \(day(deadline)) · noch \(days) Tage"),
                          systemImage: "bell.badge")
                        .appFont(.caption, weight: .semibold).foregroundStyle(Theme.warning)
                } else {
                    Label("Kündigen bis \(day(deadline)) · Ende \(day(s.end))", systemImage: "calendar")
                        .appFont(.caption).foregroundStyle(Theme.secondary)
                }
            } else if s.daysToEnd == 0 {
                Text("Vertrag endet heute").appFont(.caption).foregroundStyle(Theme.secondary)
            } else {
                Text("Läuft bis \(day(s.end))").appFont(.caption).foregroundStyle(Theme.secondary)
            }
        } else if d.cancelled {
            Label("Gekündigt", systemImage: "xmark.circle").appFont(.caption).foregroundStyle(Theme.secondary)
        } else {
            Text("\(item.series?.intervalText ?? item.group.title)").appFont(.caption).foregroundStyle(Theme.secondary)
        }
    }
}

/// Trial end and price rise of a subscription, each in its own line.
struct SubscriptionExtraLines: View {
    @Environment(AppModel.self) private var model
    let item: SubscriptionItem

    private var priceChange: SubscriptionPriceChange? {
        let newestFirst = (model.subscriptionBookings[item.id] ?? []).sorted { $0.bookingDate > $1.bookingDate }
        return SubscriptionPriceChange.detect(newestFirst.map(\.amountCents))
    }

    var body: some View {
        if let end = item.trialEndDate {
            let days = Calendar.current.dateComponents([.day], from: Calendar.current.startOfDay(for: Date()), to: end).day ?? 0
            Label("Probezeit bis \(DateText.medium(DateText.iso(end))) · danach \(Money.format(item.amountCents, currency: item.currency)) / \(item.group.perPeriod)",
                  systemImage: "gift")
                .appFont(.caption, weight: days <= 7 ? .semibold : .regular)
                .foregroundStyle(days <= 7 ? Theme.warning : Theme.secondary)
                .sensitive(.amounts)
        }
        if item.details.cancelled, let end = item.status?.end,
           let late = (model.subscriptionBookings[item.id] ?? []).map(\.bookingDate).filter({ $0 > DateText.iso(end) }).max() {
            Label("Nach Vertragsende noch gebucht am \(DateText.medium(late))", systemImage: "exclamationmark.triangle")
                .appFont(.caption).foregroundStyle(Theme.warning)
        }
        if let c = priceChange {
            Label("Preis erhöht: \(Money.format(c.old, currency: item.currency)) → \(Money.format(c.new, currency: item.currency))",
                  systemImage: "arrow.up.right.circle")
                .appFont(.caption).foregroundStyle(Theme.warning)
                .sensitive(.amounts)
        }
    }
}

/// One subscription in the list: icon, name, contract status, amount. A click selects it; its fields and
/// bookings are in the inspector on the right. A coloured bar marks a deadline within 30 days (orange) and
/// a cancelled contract that still runs (green); a contract that is over is struck through.
struct SubscriptionRow: View {
    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(\.negativeAmountStyle) private var amountStyle
    @Environment(AppModel.self) private var model
    let item: SubscriptionItem
    let basis: SubscriptionAmountBasis

    private var selected: Bool { model.selectedSubscriptionID == item.id }

    var body: some View {
        let attention = item.attention
        VStack(alignment: .leading, spacing: 0) {
            // a button, so the keyboard can reach and select the row
            Button {
                model.selectedSubscriptionID = item.id
                #if os(iOS)
                model.showBookingSheet = true
                #endif
            } label: {
                rowContent(attention)
            }
            .buttonStyle(.plain)
            Divider()
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
    }

    private func rowContent(_ attention: SubscriptionAttention) -> some View {
        let layout = typeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
            : AnyLayout(HStackLayout(alignment: .top, spacing: 12))
        return layout {
            HStack(alignment: .top, spacing: 12) {
                SubscriptionIcon(item: item).opacity(item.isEnded ? 0.5 : 1)
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        Text(item.name).appFont(.body, weight: .semibold).lineLimit(typeSize.isAccessibilitySize ? nil : 1).sensitive()
                            .strikethrough(item.isEnded, color: Theme.tertiary)
                            .foregroundStyle(item.isEnded ? Theme.secondary : Theme.text)
                        if item.isManual {
                            Text("Manuell").appFont(.caption2).foregroundStyle(Theme.secondary)
                                .padding(.horizontal, 5).padding(.vertical, 1)
                                .background(Theme.card, in: Capsule())
                        }
                    }
                    SubscriptionContractLine(item: item)
                    SubscriptionExtraLines(item: item)
                    if !item.details.note.isEmpty {
                        Text(item.details.note).appFont(.caption).foregroundStyle(Theme.secondary)
                            .lineLimit(1).sensitive()
                    }
                }
            }
            if !typeSize.isAccessibilitySize { Spacer(minLength: 8) }
            amounts
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, 10).padding(.horizontal, 10)
        .background(background(attention), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(alignment: .leading) {
            if let color = attention.color {
                Capsule().fill(color).frame(width: 3).padding(.vertical, 8).padding(.leading, 2)
            }
        }
        .contentShape(Rectangle())
    }

    private func background(_ attention: SubscriptionAttention) -> Color {
        if selected { return Theme.selection }
        return attention.color?.opacity(0.10) ?? Color.clear
    }

    private var amounts: some View {
        let ended = item.isEnded
        return VStack(alignment: typeSize.isAccessibilitySize ? .leading : .trailing, spacing: 2) {
            SubscriptionAmount(text: Money.format(item.cents(in: basis), currency: item.currency), period: item.period(in: basis))
                .appFont(.body, weight: .semibold, monospacedDigit: true)
                .foregroundStyle(ended ? Theme.secondary : amountStyle.ink(item.amountCents))
                .sensitive(.amounts)
            if basis == .original {
                if item.group != .yearly {
                    SubscriptionAmount(text: Money.format(item.yearlyCents, currency: item.currency), period: String(localized: "Jahr"))
                        .appFont(.caption, monospacedDigit: true).foregroundStyle(Theme.secondary)
                        .sensitive(.amounts)
                }
            } else {
                // the year next to the converted amount (the month next to a yearly one)
                let second = basis == .yearly ? SubscriptionAmountBasis.monthly : SubscriptionAmountBasis.yearly
                SubscriptionAmount(text: Money.format(item.cents(in: second), currency: item.currency), period: second.period ?? "")
                    .appFont(.caption, monospacedDigit: true).foregroundStyle(Theme.secondary)
                    .sensitive(.amounts)
            }
            if let due = item.nextDue, item.isActive {
                Text("nächste am \(DateText.medium(due))").appFont(.caption).foregroundStyle(Theme.tertiary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}


// A period can wrap to its own line while the formatted amount stays together.
struct SubscriptionAmount: View {
    let text: String
    let period: String

    var body: some View {
        ViewThatFits(in: .horizontal) {
            Text("\(text) / \(period)").fixedSize()
            VStack(alignment: .leading, spacing: 0) {
                Text(verbatim: text).fixedSize()
                Text(verbatim: period)
            }
        }
    }
}
