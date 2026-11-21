import SwiftUI
import GiroCrabCore

/// Pairs of a bank booking and an imported MoneyMoney row that may be the same
/// booking. The user taps the entry whose texts stay (green, "bleibt"); the
/// other is shown red ("wird entfernt"); category, notes and tags of both are
/// kept by the core. Nothing is merged without the user's click.
struct DuplicatesSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    /// Entry whose texts stay, per pair (`true` = the bank's). Unset = the bank's:
    /// later fetches deliver its wording, and MoneyMoney's may be a raw format.
    @State private var keepsBank: [String: Bool] = [:]

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Mögliche Doppel").appFont(.headline)
                Spacer()
                Button("Fertig") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding([.horizontal, .top], 16)
            // Above the list, not as its footer: a List footer is cut off on macOS.
            Text("Gleiches Konto, gleicher Betrag, höchstens drei Tage auseinander. Tippen Sie den Eintrag an, dessen Texte bleiben sollen. Nichts, was Sie eingetragen haben, geht verloren: Eine von Hand vergebene Kategorie gewinnt, Notizen werden zusammengefügt, Tags beider bleiben. „wird übernommen“ zeigt, was vom entfernten Eintrag mitkommt.")
                .appFont(.caption)
                .foregroundStyle(Theme.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
            Divider()
            List {
                Section {
                    if model.possibleDuplicates.isEmpty {
                        Text("Keine möglichen Doppel gefunden.")
                            .foregroundStyle(Theme.secondary)
                    }
                    ForEach(model.possibleDuplicates, id: \.self) { pair in
                        row(pair)
                    }
                } header: {
                    Text("Paare (\(model.possibleDuplicates.count))")
                }
            }
        }
        .desktopFrame(minWidth: 620, idealWidth: 720, minHeight: 520, idealHeight: 640)
        #if os(macOS)
        .onExitCommand { dismiss() }
        #endif
    }

    private static func key(_ pair: PossibleDuplicate) -> String {
        "\(pair.bank.bookingId)-\(pair.imported.bookingId)"
    }

    private func keeps(_ pair: PossibleDuplicate) -> Bool {
        keepsBank[Self.key(pair)] ?? true
    }

    private func row(_ pair: PossibleDuplicate) -> some View {
        let keepBank = keeps(pair)
        return VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(pair.accountName)
                    .appFont(.caption)
                    .foregroundStyle(Theme.secondary)
                    .sensitive()
                Spacer()
                Text(Money.format(pair.amountCents, currency: pair.currency))
                    .sensitive(.amounts)
                    .monospacedDigit()
                    .foregroundStyle(pair.amountCents < 0 ? Theme.outflow : Theme.inflow)
            }
            choice(pair.bank, symbol: "building.columns", label: "Bankabruf", kept: keepBank,
                   marks: Self.marks(pair.bank, kept: keepBank, partner: pair.imported)) {
                keepsBank[Self.key(pair)] = true
            }
            choice(pair.imported, symbol: "square.and.arrow.down", label: "Import aus MoneyMoney", kept: !keepBank,
                   marks: Self.marks(pair.imported, kept: !keepBank, partner: pair.bank)) {
                keepsBank[Self.key(pair)] = false
            }
            HStack {
                Spacer()
                Button("Kein Doppel") { model.dismissDuplicate(pair) }
                Button("Zusammenführen") { model.mergeDuplicate(pair, keepBank: keepBank) }
                    .buttonStyle(.borderedProminent)
            }
        }
        .padding(.vertical, 4)
    }

    /// What happens to an entry's category, note and tags on merge (the core's
    /// rules): the kept entry's category is replaced by one by hand on the other
    /// side; from the removed entry, a winning category, a different note and
    /// tags the kept one lacks are carried over.
    private struct Marks {
        var categoryCarried = false
        var categoryReplaced = false
        var noteCarried = false
        var tagsCarried = false
    }

    private static func rank(_ s: DuplicateSide) -> Int {
        s.categoryName.isEmpty ? 0 : (s.categoryByHand ? 2 : 1)
    }

    private static func marks(_ s: DuplicateSide, kept: Bool, partner: DuplicateSide) -> Marks {
        var m = Marks()
        if kept {
            // "Ohne Kategorie" is not replaced, it is filled: the partner's line says so
            m.categoryReplaced = !s.categoryName.isEmpty && rank(partner) > rank(s)
        } else {
            m.categoryCarried = rank(s) > rank(partner)
            let note = s.note.trimmingCharacters(in: .whitespaces)
            m.noteCarried = !note.isEmpty && note != partner.note.trimmingCharacters(in: .whitespaces)
            m.tagsCarried = s.tags.contains { !partner.tags.contains($0) }
        }
        return m
    }

    /// "wird übernommen" / "wird ersetzt" behind a line.
    @ViewBuilder
    private func mark(carried: Bool, replaced: Bool = false) -> some View {
        if carried {
            Text("wird übernommen").foregroundStyle(Theme.inflow)
        } else if replaced {
            Text("wird ersetzt").foregroundStyle(Theme.negative)
        }
    }

    /// Category, note and tags of one entry, always shown: the comparison is
    /// what the user decides on; marks say what moves on merge.
    private func details(_ s: DuplicateSide, marks: Marks) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 4) {
                Image(systemName: "folder").accessibilityHidden(true)
                Text(s.categoryName.isEmpty ? String(localized: "Ohne Kategorie") : s.categoryName).sensitive(.labels)
                if !s.categoryName.isEmpty {
                    Text(s.categoryByHand ? LocalizedStringKey("(von Hand)") : LocalizedStringKey("(Regel)"))
                        .foregroundStyle(Theme.tertiary)
                }
                mark(carried: marks.categoryCarried, replaced: marks.categoryReplaced)
            }
            if !s.note.isEmpty {
                HStack(alignment: .top, spacing: 4) {
                    Image(systemName: "note.text").accessibilityHidden(true)
                    Text(s.note).lineLimit(2).sensitive()
                    mark(carried: marks.noteCarried)
                }
            }
            if !s.tags.isEmpty {
                HStack(spacing: 4) {
                    Image(systemName: "tag").accessibilityHidden(true)
                    Text(s.tags.joined(separator: ", ")).lineLimit(1).sensitive(.labels)
                    mark(carried: marks.tagsCarried)
                }
            }
        }
        .appFont(.caption)
        .foregroundStyle(Theme.secondary)
    }

    /// One entry as a large tap target: green with a check mark when it stays,
    /// red and dimmed when it goes. Colour is never the only cue (mark + text).
    private func choice(
        _ s: DuplicateSide, symbol: String, label: LocalizedStringKey, kept: Bool, marks: Marks,
        select: @escaping () -> Void
    ) -> some View {
        Button(action: select) {
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: kept ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(kept ? Theme.inflow : Theme.tertiary)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
                        Image(systemName: symbol)
                            .accessibilityHidden(true)
                        Text(label)
                    }
                    .appFont(.caption)
                    .foregroundStyle(Theme.tertiary)
                    HStack(spacing: 6) {
                        Text(DateText.medium(s.bookingDate)).monospacedDigit()
                        Text(s.counterparty.isEmpty ? String(localized: "Ohne Name") : s.counterparty)
                            .lineLimit(1)
                            .sensitive()
                    }
                    Text(s.purpose)
                        .appFont(.caption)
                        .foregroundStyle(Theme.secondary)
                        .lineLimit(2)
                        .sensitive()
                    details(s, marks: marks)
                        .padding(.top, 3)
                }
                Spacer(minLength: 8)
                Text(kept ? "bleibt" : "wird entfernt")
                    .appFont(.caption)
                    .foregroundStyle(kept ? Theme.inflow : Theme.negative)
            }
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .fill((kept ? Theme.inflow : Theme.negative).opacity(0.12))
            )
            .opacity(kept ? 1 : 0.7)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(kept ? .isSelected : [])
        .accessibilityHint(kept ? Text("Dieser Eintrag bleibt") : Text("Antippen, um diesen Eintrag zu behalten"))
    }
}
