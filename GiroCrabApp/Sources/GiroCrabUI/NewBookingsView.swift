import SwiftUI
import GiroCrabCore

/// „Neue Umsätze“ (GitHub #352): one card per account with unreviewed bookings,
/// each listing only those. Shown inside `BookingStreamView` in place of the
/// day cards, so toolbar, keyboard, inspector sheet, print and paging are the
/// booking list's own (`currentFilter` asks for unchecked bookings).
struct NewBookingsCards: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let sections = model.newBookingSections
        if sections.isEmpty, !model.isLoadingSelection {
            ContentUnavailableView(
                "Keine neuen Umsätze",
                systemImage: "checkmark.circle",
                description: Text("Alle Buchungen sind geprüft.")
            )
            .frame(maxWidth: .infinity)
            .padding(.top, 40)
        }
        ForEach(sections) { section in
            NewBookingsCardHeader(section: section)
                .padding(.top, 14)
            ForEach(section.bookings) { b in
                DayCardRow(booking: b, isLast: b.id == section.bookings.last?.id)
                    .id(StreamSlot.newBooking(b.id))
            }
        }
    }
}

/// Top of an account card: signet, name, IBAN · bank, balance and pending sum.
private struct NewBookingsCardHeader: View {
    @Environment(AppModel.self) private var model
    #if os(iOS)
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    #endif
    let section: NewBookingsSection

    private var stacksSummary: Bool {
        #if os(iOS)
        horizontalSizeClass == .compact && dynamicTypeSize.isAccessibilitySize
        #else
        false
        #endif
    }

    private var detail: String {
        guard let a = section.account else { return "" }
        let number = a.iban.isEmpty ? a.accountNumber : formatIBAN(a.iban)
        return [number, a.bankName].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    var body: some View {
        let stacked = stacksSummary
        let summaryLayout = stacked
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 6))
            : AnyLayout(HStackLayout(alignment: .top, spacing: 12))
        VStack(spacing: 0) {
            // signet size, gap and inset match `BookingRow`, so the name lines up with the rows below (#1556)
            HStack(alignment: .top, spacing: 12) {
                if let a = section.account {
                    AccountSignet(account: a, size: 36, color: model.accountColor(a.id), onSelection: false)
                }
                summaryLayout {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(section.title)
                            .appFont(.headline)
                            .sensitive()
                            .foregroundStyle(Theme.text)
                            .lineLimit(stacked ? nil : 1)
                        if !detail.isEmpty {
                            Text(detail)
                                .appFont(.caption)
                                .sensitive()
                                .foregroundStyle(Theme.secondary)
                                .lineLimit(1)
                        }
                    }
                    if !stacked { Spacer() }
                    VStack(alignment: stacked ? .leading : .trailing, spacing: 2) {
                        if let a = section.account {
                            Text(Money.format(a.balanceCents, currency: a.currency))
                                .appFont(.headline, monospacedDigit: true)
                                .fixedSize() // keep the balance and currency together (#343)
                                .sensitive(.amounts)
                                .foregroundStyle(a.balanceCents < 0 ? Theme.negative : Theme.text)
                        }
                        if section.pendingCents != 0 {
                            Text("Vorgemerkt: \(Money.format(section.pendingCents, currency: section.account?.currency ?? "EUR"))")
                                .appFont(.caption, monospacedDigit: true)
                                .fixedSize(horizontal: !stacked, vertical: true)
                                .sensitive(.amounts)
                                .foregroundStyle(Theme.secondary)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            Rectangle().fill(Theme.border).frame(height: 1)
        }
        .modifier(DayCardSegment())
    }
}
