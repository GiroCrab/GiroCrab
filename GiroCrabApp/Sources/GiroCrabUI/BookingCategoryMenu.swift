import SwiftUI
import GiroCrabCore

/// Bookings dragged from the list onto a sidebar category (#366, macOS only:
/// iOS has no drag pasteboard to tell them from accounts). A plain string like
/// `GroupDrag`, so the sidebar's drop zones receive it. Every other `String`
/// or `URL` drop target sees it too and must ignore it. The row drags it as
/// part of `BookingDragItem`, next to the JSON for other apps.
enum BookingDrag {
    static let prefix = "girocrab-bookings:"

    static func payload(_ ids: [Int64]) -> String {
        prefix + ids.map(String.init).joined(separator: ",")
    }

    static func ids(_ s: String) -> [Int64] {
        guard s.hasPrefix(prefix) else { return [] }
        return s.dropFirst(prefix.count).split(separator: ",").compactMap { Int64($0) }
    }

    /// Whether the drag in progress carries bookings.
    static var isDragged: Bool { GroupDrag.draggedString?.hasPrefix(prefix) == true }
}

/// "Kategorie" of the row menu (#366): the category tree to pick from, the
/// search sheet and removal. Acts on the multiple selection when the row is
/// part of it. A view of its own, so the selection and the categories stay out
/// of the day card's dependencies.
struct CategoryAssignMenu: View {
    @Environment(AppModel.self) private var model
    let booking: Booking

    var body: some View {
        let targets = model.categorizeTargets(for: booking)
        let assigned = Set(targets.map(\.categoryId))
        Menu("Kategorie", systemImage: "tag") {
            if targets.count == 1, booking.categoryId == nil, let s = model.listSuggestions[booking.id],
               let category = model.categories.first(where: { $0.id == s.categoryId }) {
                Button("Vorschlag übernehmen: \(category.name)", systemImage: "clock.arrow.circlepath") {
                    model.setCategory(category.id, for: targets)
                }
                Divider()
            }
            CategoryMenuItems(
                nodes: model.categoryTree,
                parent: nil,
                current: assigned.count == 1 ? assigned.first ?? nil : nil,
                pick: { model.setCategory($0, for: targets) },
                create: { parent in
                    model.newCategoryRequest = NewCategoryRequest(parent: parent, bookingIDs: targets.map(\.id))
                }
            )
            Divider()
            Button(targets.count > 1 ? "\(targets.count) Buchungen kategorisieren …" : "Kategorisieren …",
                   systemImage: "magnifyingglass") {
                if !model.selectedBookingIDs.contains(booking.id) { model.selectedBookingID = booking.id }
                model.showCategorizer = true
            }
            Button("Kategorie entfernen", systemImage: "tag.slash") { model.setCategory(nil, for: targets) }
                .disabled(assigned == [nil])
        }
    }
}

/// The category tree as nested menus: a category with sub-categories opens
/// them, a leaf is assigned (as in the inspector's grid). `current` is checked;
/// picking it again changes nothing (it would turn a rule's category manual).
private struct CategoryMenuItems: View {
    let nodes: [CategoryNode]
    let parent: Int64?
    let current: Int64?
    let pick: (Int64) -> Void
    let create: (Int64?) -> Void

    var body: some View {
        ForEach(nodes) { node in
            let label = Label(node.category.name, systemImage: node.category.sfSymbol)
            if let kids = node.children {
                Menu {
                    CategoryMenuItems(nodes: kids, parent: node.id, current: current, pick: pick, create: create)
                } label: { label }
            } else {
                Toggle(isOn: Binding(get: { node.id == current }, set: { if $0 { pick(node.id) } })) { label }
            }
        }
        if !nodes.isEmpty { Divider() }
        Button("Neue Kategorie …", systemImage: "plus") { create(parent) }
    }
}

#if os(macOS)
/// The inline picker displays the sidebar's category order in one scroll view.
struct BookingCategoryPicker: View {
    @Environment(AppModel.self) private var model
    @Environment(\.textScale) private var textScale
    let booking: Booking
    let dismiss: () -> Void

    struct Row: Identifiable {
        let category: GiroCrabCore.Category
        let depth: Int
        let isGroup: Bool
        var id: Int64 { category.id }
    }

    static func rows(for nodes: [CategoryNode], depth: Int = 0) -> [Row] {
        nodes.flatMap { node in
            let row = Row(category: node.category, depth: depth, isGroup: node.children != nil)
            return [row] + rows(for: node.children ?? [], depth: depth + 1)
        }
    }

    private var suggestedCategory: GiroCrabCore.Category? {
        guard booking.categoryId == nil, let suggestion = model.listSuggestions[booking.id] else { return nil }
        return model.categories.first { $0.id == suggestion.categoryId }
    }

    var body: some View {
        let rows = Self.rows(for: model.categoryTree)
        let suggestion = suggestedCategory
        let entryCount = rows.count + (suggestion == nil ? 1 : 2)
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                    Button {
                        choose(nil)
                    } label: {
                        option(title: model.uncategorizedLabel, symbol: "questionmark.folder",
                               color: Theme.warning, selected: booking.categoryId == nil)
                    }

                    if let category = suggestion {
                        Button {
                            choose(category.id)
                        } label: {
                            option(title: String(localized: "Vorschlag übernehmen: \(category.name)"),
                                   symbol: "clock.arrow.circlepath", color: Theme.secondary)
                        }
                        Divider()
                    }

                    ForEach(rows) { row in
                        categoryRow(row)
                    }
                }
                .buttonStyle(.plain)
                .padding(6)
            }
            .frame(width: 238 * textScale, height: min(360 * textScale, 12 + CGFloat(entryCount) * 26 * textScale))
            .onAppear {
                if let id = booking.categoryId { proxy.scrollTo(id, anchor: .center) }
            }
        }
    }

    @ViewBuilder
    private func categoryRow(_ row: Row) -> some View {
        let category = row.category
        let content = option(title: category.name, symbol: category.sfSymbol,
                             color: Color.readable(hex: category.colorHex),
                             selected: booking.categoryId == row.id, depth: row.depth)
        if row.isGroup {
            content.accessibilityAddTraits(.isHeader)
        } else {
            Button { choose(row.id) } label: { content }
        }
    }

    private func option(title: String, symbol: String, color: Color, selected: Bool = false,
                        depth: Int = 0) -> some View {
        HStack(spacing: 7) {
            Image(systemName: symbol)
                .foregroundStyle(color)
                .frame(width: 16 * textScale)
                .accessibilityHidden(true)
            Text(title)
                .foregroundStyle(color)
                .sensitive(.labels)
                .lineLimit(1)
            Spacer(minLength: 4)
            if selected {
                Image(systemName: "checkmark")
                    .foregroundStyle(Theme.accent)
                    .accessibilityHidden(true)
            }
        }
        .appFont(.callout)
        .padding(.leading, 6 + CGFloat(depth) * 12 * textScale)
        .padding(.trailing, 8)
        .padding(.vertical, 4)
        .background {
            if selected {
                RoundedRectangle(cornerRadius: 5).fill(Theme.accent.opacity(0.14))
            }
        }
        .contentShape(Rectangle())
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func choose(_ id: Int64?) {
        dismiss()
        if id != booking.categoryId { model.setCategory(id, for: booking) }
    }
}
#endif
