import SwiftUI
import GiroCrabCore

/// „Abos“ (GitHub #1787): recurring expenses and incomes the user marked as subscription or contract, plus
/// manual entries. Divided into expenses and incomes, then by rhythm or by active / inactive. The amounts are
/// shown in the own rhythm or converted to one period; the contract fields live in the inspector.
/// Design page: `docs/ABOS_UI.md`.
struct SubscriptionsView: View {
    @Environment(\.negativeAmountStyle) private var amountStyle
    @Environment(AppModel.self) private var model
    @AppStorage("subscriptions.sort") private var sortRaw = SubscriptionSort.name.rawValue
    @AppStorage("subscriptions.grouping") private var groupingRaw = SubscriptionGrouping.rhythm.rawValue
    @AppStorage("subscriptions.basis") private var basisRaw = SubscriptionAmountBasis.original.rawValue
    @State private var search = ""
    @State private var statusFilter = SubscriptionStatusFilter.all
    @State private var kindFilter = SubscriptionKindFilter.all
    @State private var rhythmFilter: SubscriptionGroup?
    @AppStorage("subscriptions.hideEnded") private var hideEnded = false
    @State private var showAdd = false

    private var sort: SubscriptionSort { SubscriptionSort(rawValue: sortRaw) ?? .name }
    private var grouping: SubscriptionGrouping { SubscriptionGrouping(rawValue: groupingRaw) ?? .rhythm }
    private var basis: SubscriptionAmountBasis { SubscriptionAmountBasis(rawValue: basisRaw) ?? .original }
    private var activeFilters: Int {
        (kindFilter == .all ? 0 : 1) + (rhythmFilter == nil ? 0 : 1) + (statusFilter == .all ? 0 : 1) + (hideEnded ? 1 : 0)
    }

    private func shown(_ all: [SubscriptionItem]) -> [SubscriptionItem] {
        let q = search.trimmingCharacters(in: .whitespaces)
        return all.filter { i in
            statusFilter.matches(i) && kindFilter.matches(i)
                && !(hideEnded && i.isEnded && statusFilter != .inactive)
                && (rhythmFilter == nil || i.group == rhythmFilter)
                && (q.isEmpty || i.name.localizedCaseInsensitiveContains(q) || i.details.note.localizedCaseInsensitiveContains(q))
        }
    }

    var body: some View {
        let all = model.subscriptionItems
        let items = shown(all)
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Abos & Verträge").appFont(.title2, weight: .semibold)
                Text("Wiederkehrende Ausgaben und Einnahmen, die du als Abo oder Vertrag markiert hast, und selbst angelegte Einträge. Die Beträge stehen im eigenen Turnus oder umgerechnet.")
                    .appFont(.caption).foregroundStyle(Theme.secondary).fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 20)
            controls.padding(.horizontal, 20)
            if !items.isEmpty { totals(items).padding(.horizontal, 20) }
            if all.isEmpty {
                ContentUnavailableView {
                    Label("Keine Abos", systemImage: "repeat.circle")
                } description: {
                    Text("Wähle erkannte wiederkehrende Buchungen aus oder lege ein Abo von Hand an. Verträge beim selben Anbieter erscheinen nur dann als eigene Einträge, wenn sich ihre Beträge deutlich unterscheiden.")
                } actions: {
                    Button("Abo hinzufügen …") { showAdd = true }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if items.isEmpty {
                ContentUnavailableView("Nichts gefunden", systemImage: "magnifyingglass",
                                       description: Text("Kein Abo passt zu Suche und Filtern."))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                list(items)
            }
        }
        .padding(.top, 16)
        .background(Theme.canvas)
        .task {
            model.refreshRecurring()
            model.seedDemoSubscriptions()
            await model.reloadSubscriptionBookings()
        }
        .capturableSheet(isPresented: $showAdd) { SubscriptionAddSheet() }
    }

    // MARK: controls

    /// One row when it fits, else search and button above, the menus below (the column is narrow
    /// next to the sidebar and the inspector).
    private var controls: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) {
                searchField
                sortMenu
                basisMenu
                filterMenu
                Spacer(minLength: 0)
                addButton
            }
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 10) {
                    searchField
                    Spacer(minLength: 0)
                    addButton
                }
                HStack(spacing: 10) {
                    sortMenu
                    basisMenu
                    filterMenu
                    Spacer(minLength: 0)
                }
            }
        }
    }

    private var searchField: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").foregroundStyle(Theme.tertiary).accessibilityHidden(true)
            TextField("Suchen", text: $search).textFieldStyle(.plain).appFont(.callout)
            if !search.isEmpty {
                Button { search = "" } label: { Image(systemName: "xmark.circle.fill") }
                    .buttonStyle(.plain).foregroundStyle(Theme.tertiary)
                    .accessibilityLabel("Suche löschen")
            }
        }
        .padding(.horizontal, 8).padding(.vertical, 5)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        .frame(minWidth: 140, maxWidth: 240)
    }

    private var sortMenu: some View {
        Menu {
            Picker("Sortieren", selection: $sortRaw) {
                ForEach(SubscriptionSort.allCases) { Text($0.title).tag($0.rawValue) }
            }
            .pickerStyle(.inline)
            Picker("Gruppieren", selection: $groupingRaw) {
                ForEach(SubscriptionGrouping.allCases) { Text($0.title).tag($0.rawValue) }
            }
            .pickerStyle(.inline)
        } label: {
            Label(sort.title, systemImage: "arrow.up.arrow.down")
        }
        .fixedSize()
        .help("Sortieren und gruppieren")
    }

    private var basisMenu: some View {
        Menu {
            Picker("Beträge anzeigen", selection: $basisRaw) {
                ForEach(SubscriptionAmountBasis.allCases) { Text($0.title).tag($0.rawValue) }
            }
            .pickerStyle(.inline)
        } label: {
            Label(basis.shortTitle, systemImage: "eurosign.circle")
        }
        .fixedSize()
        .help("Beträge im eigenen Turnus oder umgerechnet anzeigen")
    }

    private var filterMenu: some View {
        Menu {
            Picker("Art", selection: $kindFilter) {
                ForEach(SubscriptionKindFilter.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.inline)
            Picker("Turnus", selection: $rhythmFilter) {
                Text("Alle Turnusse").tag(SubscriptionGroup?.none)
                ForEach(SubscriptionGroup.allCases) { Text($0.title).tag(SubscriptionGroup?.some($0)) }
            }
            .pickerStyle(.inline)
            Picker("Status", selection: $statusFilter) {
                ForEach(SubscriptionStatusFilter.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.inline)
            Toggle("Beendete ausblenden", isOn: $hideEnded)
            if activeFilters > 0 {
                Divider()
                Button("Filter zurücksetzen") { kindFilter = .all; rhythmFilter = nil; statusFilter = .all; hideEnded = false }
            }
        } label: {
            Label(activeFilters == 0 ? String(localized: "Filter") : String(localized: "Filter (\(activeFilters))"),
                  systemImage: activeFilters == 0 ? "line.3.horizontal.decrease.circle" : "line.3.horizontal.decrease.circle.fill")
        }
        .fixedSize()
        .help("Nach Art, Turnus und Status filtern")
    }

    private var addButton: some View {
        Button { showAdd = true } label: { Label("Abo hinzufügen …", systemImage: "plus") }
            .fixedSize()
    }

    // MARK: totals

    /// Sums of the active subscriptions: expenses and incomes side by side, per column the amount per
    /// month above the amount per year (like the header of the inspector), per currency.
    private func totals(_ items: [SubscriptionItem]) -> some View {
        let active = items.filter(\.isActive)
        return VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 32) {
                ForEach([false, true], id: \.self) { income in
                    let part = active.filter { $0.isIncome == income }
                    if !part.isEmpty { totalColumn(income, part) }
                }
                Spacer(minLength: 0)
            }
            Text("Aktive Abos: \(active.count) · beendete nicht eingerechnet")
                .appFont(.caption).foregroundStyle(Theme.secondary)
        }
    }

    private func totalColumn(_ income: Bool, _ part: [SubscriptionItem]) -> some View {
        // „Original“ has no single period for a sum: per month, like the year below it
        let main = basis == .original ? SubscriptionAmountBasis.monthly : basis
        let second = main == .yearly ? SubscriptionAmountBasis.monthly : SubscriptionAmountBasis.yearly
        return VStack(alignment: .leading, spacing: 3) {
            Text(income ? "Einnahmen" : "Ausgaben").appFont(.callout, weight: .semibold)
            ForEach(AppModel.subscriptionTotals(part) { $0.cents(in: main) }, id: \.currency) { sum in
                Text("\(Money.format(sum.cents, currency: sum.currency)) / \(main.period ?? "")")
                    .appFont(.title3, weight: .semibold, monospacedDigit: true)
                    .foregroundStyle(amountStyle.ink(sum.cents)).sensitive(.amounts).fixedSize()
            }
            ForEach(AppModel.subscriptionTotals(part) { $0.cents(in: second) }, id: \.currency) { sum in
                Text("\(Money.format(sum.cents, currency: sum.currency)) / \(second.period ?? "")")
                    .appFont(.callout, monospacedDigit: true)
                    .foregroundStyle(Theme.secondary).sensitive(.amounts).fixedSize()
            }
        }
    }

    // MARK: list

    private func list(_ items: [SubscriptionItem]) -> some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach([false, true], id: \.self) { income in
                    let part = items.filter { $0.isIncome == income }
                    if !part.isEmpty {
                        SubscriptionKindHeader(income: income, count: part.count)
                        sections(part)
                    }
                }
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 18)
        }
    }

    @ViewBuilder private func sections(_ part: [SubscriptionItem]) -> some View {
        switch grouping {
        case .rhythm:
            ForEach(SubscriptionGroup.allCases) { group in
                let members = sort.sorted(part.filter { $0.group == group })
                if !members.isEmpty {
                    SubscriptionGroupHeader(title: group.title, items: members, basis: basis, rhythm: group)
                    ForEach(members) { SubscriptionRow(item: $0, basis: basis) }
                }
            }
        case .activity:
            ForEach([true, false], id: \.self) { active in
                let members = sort.sorted(part.filter { $0.isActive == active })
                if !members.isEmpty {
                    SubscriptionGroupHeader(title: active ? String(localized: "Aktiv") : String(localized: "Inaktiv"),
                                            items: members, basis: basis, rhythm: nil)
                    ForEach(members) { SubscriptionRow(item: $0, basis: basis) }
                }
            }
        case .none:
            ForEach(sort.sorted(part)) { SubscriptionRow(item: $0, basis: basis) }
        }
    }
}

/// „Ausgaben“ / „Einnahmen“ above the groups.
private struct SubscriptionKindHeader: View {
    let income: Bool
    let count: Int

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: income ? "arrow.down.circle" : "arrow.up.circle")
                .foregroundStyle(income ? Theme.inflow : Theme.secondary).accessibilityHidden(true)
            Text(income ? "Einnahmen" : "Ausgaben").appFont(.title3, weight: .semibold)
            Text("(\(count))").appFont(.caption).foregroundStyle(Theme.secondary)
            Spacer(minLength: 0)
        }
        .padding(.top, 18)
        .accessibilityElement(children: .combine)
    }
}

/// "Vierteljährlich (1)" with the sum of its active subscriptions. In the own rhythm a rhythm group
/// shows its sum per payment and per year; every other combination one converted amount and the year.
private struct SubscriptionGroupHeader: View {
    let title: String
    let items: [SubscriptionItem]
    let basis: SubscriptionAmountBasis
    let rhythm: SubscriptionGroup?

    private var sums: [(text: String, cents: [(currency: String, cents: Int64)])] {
        let active = items.filter(\.isActive)
        if active.isEmpty { return [] }
        if basis == .original, let rhythm {
            guard rhythm != .custom else { return [] }
            var out = [(rhythm.perPeriod, AppModel.subscriptionTotals(active, cents: \.amountCents))]
            if rhythm != .yearly { out.append((String(localized: "Jahr"), AppModel.subscriptionTotals(active, cents: \.yearlyCents))) }
            return out
        }
        let b = basis == .original ? SubscriptionAmountBasis.monthly : basis
        var out = [(b.period ?? "", AppModel.subscriptionTotals(active) { $0.cents(in: b) })]
        if b != .yearly { out.append((String(localized: "Jahr"), AppModel.subscriptionTotals(active, cents: \.yearlyCents))) }
        return out
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(title).appFont(.headline)
            Text("(\(items.count))").appFont(.caption).foregroundStyle(Theme.secondary)
            Spacer(minLength: 8)
            ForEach(Array(sums.enumerated()), id: \.offset) { index, sum in
                if index > 0 { Text(verbatim: "·").foregroundStyle(Theme.tertiary) }
                Text(verbatim: "\(Self.joined(sum.cents)) / \(sum.text)")
                    .appFont(.callout, monospacedDigit: true).foregroundStyle(Theme.secondary)
                    .sensitive(.amounts)
            }
        }
        .padding(.top, 14).padding(.bottom, 6)
        .overlay(alignment: .bottom) { Divider() }
        .accessibilityElement(children: .combine)
    }

    static func joined(_ sums: [(currency: String, cents: Int64)]) -> String {
        sums.map { Money.format($0.cents, currency: $0.currency) }.joined(separator: " + ")
    }
}
