import SwiftUI
import GiroCrabCore

// Recipient entry shared by the transfer form and the standing order form (#1475): the list of
// known recipients while typing (#881, #978) and the Umbuchung to an own account (#547, #582).
// The rules for the own accounts live in `AppModel.ownTransferTargets(from:)`.

/// State of the suggestion list under a recipient name field.
@MainActor @Observable
final class RecipientSuggestions {
    var items: [KnownRecipient] = []
    /// Row chosen with the arrow keys or the pointer.
    var highlighted: Int?
    /// Row to highlight once the list has changed because one was hidden (#978): the next one
    /// moves into its place. Any other change of the list (typing) clears the highlight.
    var keepHighlight: Int?
    /// Name just taken from the list: typing it must not reopen the list.
    var pickedName: String?
    /// What the field reports, so the list can be refreshed from outside (after "Empfänger verwalten").
    var name = ""
    var focused = false
    /// The person changed the name while the field had the focus. Until then (a form opened with a name,
    /// as "Dauerauftrag ändern" does) the list stays closed, although the field gets the focus.
    var edited = false

    /// Known recipients for what was typed: only once something is typed, one row per IBAN
    /// (the most used spelling), the most recently used first, at most five.
    func recipients(_ query: String, model: AppModel) -> [KnownRecipient] {
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { return [] }
        return model.suggestRecipients(query)
    }

    /// The list for the name as typed: only while the field has the focus and the name was not
    /// just taken from the list.
    func refresh(model: AppModel) {
        items = edited && focused && name != pickedName ? recipients(name, model: model) : []
    }

    /// "Nicht mehr vorschlagen": the IBAN leaves the list at once, the highlight keeps its
    /// place; "Empfänger" shows it again.
    func hide(_ r: KnownRecipient, model: AppModel) {
        let before = items.map(\.iban), place = highlighted
        model.setRecipientHidden(r.iban, true)
        refresh(model: model)
        if items.map(\.iban) != before { keepHighlight = place }
    }

    func move(_ step: Int) -> KeyPress.Result {
        guard !items.isEmpty else { return .ignored }
        highlighted = min(max((highlighted ?? -1) + step, 0), items.count - 1)
        return .handled
    }

    /// Closes the list after a row was taken; the caller copies name and IBAN into its draft.
    func take(_ r: KnownRecipient) {
        pickedName = r.name
        items = []
        highlighted = nil
    }

    /// The list changed (typed, hidden): a kept highlight moves into place, any other clears it.
    func listChanged() {
        highlighted = keepHighlight.flatMap { items.isEmpty ? nil : min($0, items.count - 1) }
        keepHighlight = nil
    }
}

/// The recipient name field with its suggestions: arrow keys move, Return picks, Esc closes.
/// The list itself floats over the form (`recipientSuggestionList`), so a row cannot clip it.
struct RecipientNameField: View {
    @Environment(AppModel.self) private var model
    @Binding var name: String
    let suggestions: RecipientSuggestions
    /// Takes the row into the draft (name and IBAN) and goes on.
    let pick: (KnownRecipient) -> Void
    @FocusState private var focused: Bool

    var body: some View {
        TextField("Empfänger", text: $name)
            .transferField()
            .focused($focused)
            .onKeyPress(.downArrow) { suggestions.move(1) }
            .onKeyPress(.upArrow) { suggestions.move(-1) }
            .onKeyPress(.return) {
                guard let h = suggestions.highlighted, suggestions.items.indices.contains(h) else { return .ignored }
                pick(suggestions.items[h])
                return .handled
            }
            .onKeyPress(.escape) {
                guard !suggestions.items.isEmpty else { return .ignored }
                suggestions.items = []
                return .handled
            }
            .sensitive()
            .anchorPreference(key: NameFieldBounds.self, value: .bounds) { $0 }
            .onAppear { suggestions.name = name }
            .onChange(of: name) { _, new in
                if new != suggestions.pickedName { suggestions.pickedName = nil }
                if focused { suggestions.edited = true }
                suggestions.name = new
                suggestions.refresh(model: model)
            }
            .onChange(of: focused) { _, now in
                suggestions.focused = now
                suggestions.refresh(model: model)
            }
    }
}

extension View {
    /// The suggestion list of a `RecipientNameField`, floating below the field.
    func recipientSuggestionList(_ suggestions: RecipientSuggestions, pick: @escaping (KnownRecipient) -> Void) -> some View {
        overlayPreferenceValue(NameFieldBounds.self) { anchor in
            GeometryReader { g in
                if let anchor, !suggestions.items.isEmpty {
                    let field = g[anchor]
                    RecipientSuggestionList(suggestions: suggestions, pick: pick)
                        .frame(width: field.width)
                        .offset(x: field.minX, y: field.maxY + 4)
                }
            }
        }
        // here, not on the list: that one only exists while it has rows, so it would miss the
        // list appearing and emptying
        .onChange(of: suggestions.items.map(\.iban)) { suggestions.listChanged() }
    }
}

/// Known recipients for what was typed: pick with the mouse or ↑/↓ and Return; Esc closes it.
/// The row under the pointer offers "Nicht mehr vorschlagen" (#978), as does its context menu.
struct RecipientSuggestionList: View {
    @Environment(AppModel.self) private var model
    let suggestions: RecipientSuggestions
    let pick: (KnownRecipient) -> Void

    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(suggestions.items.enumerated()), id: \.element.iban) { i, r in
                HStack(spacing: 4) {
                    Button { pick(r) } label: {
                        VStack(alignment: .leading, spacing: 1) {
                            HStack(alignment: .firstTextBaseline) {
                                Text(r.name).appFont(.body).lineLimit(1).sensitive()
                                Spacer(minLength: 8)
                                Text("zuletzt \(DateText.medium(r.lastUsed))").appFont(.caption)
                                    .foregroundStyle(Theme.tertiary).fixedSize()
                            }
                            RecipientIBANLine(iban: r.iban)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.leading, 8).padding(.vertical, 4)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    Button { suggestions.hide(r, model: model) } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.tertiary)
                            .padding(.trailing, 8).padding(.vertical, 4)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help("Nicht mehr vorschlagen")
                    .opacity(i == suggestions.highlighted ? 1 : 0)
                    .disabled(i != suggestions.highlighted)
                    .accessibilityHidden(true)
                }
                .background(RoundedRectangle(cornerRadius: 6).fill(i == suggestions.highlighted ? Theme.selection : .clear))
                .onHover { if $0 { suggestions.highlighted = i } }
                .contextMenu {
                    Button("Nicht mehr vorschlagen", systemImage: "eye.slash") { suggestions.hide(r, model: model) }
                }
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(i == suggestions.highlighted ? .isSelected : [])
                .accessibilityAction { pick(r) }
                .accessibilityAction(named: "Nicht mehr vorschlagen") { suggestions.hide(r, model: model) }
            }
        }
        .padding(4)
        .background(RoundedRectangle(cornerRadius: 8).fill(Theme.card))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Theme.border))
        .shadow(color: .black.opacity(0.25), radius: 8, y: 4)
    }
}

/// Umbuchung: the own account to book to; name and IBAN follow from it.
struct OwnTargetFields: View {
    let targets: [Account]
    @Binding var selection: Int64?
    /// The recipient name the draft carries.
    let name: String
    let select: (Int64?) -> Void

    var body: some View {
        Picker("Zielkonto", selection: Binding(get: { selection }, set: { selection = $0; select($0) })) {
            ForEach(targets) { a in
                Text("\(a.name) · \(formatIBAN(a.iban))").tag(Optional(a.id)).sensitive()
            }
        }
        LabeledContent("Empfänger") {
            Text(name).appFont(.body).sensitive()
        }
    }
}

/// Switch between a transfer to someone else and an Umbuchung to an own account.
struct OwnTransferSwitch: View {
    @Binding var own: Bool

    var body: some View {
        Picker("Art", selection: $own) {
            Text("Überweisung").tag(false)
            Text("Umbuchung").tag(true)
        }
        .pickerStyle(.segmented).labelsHidden().fixedSize()
        .frame(maxWidth: .infinity)
    }
}

/// The check mark or cross beside an IBAN or BIC field (nothing while the field is empty).
struct FieldValidity: View {
    let ok: Bool?
    let valid: LocalizedStringKey
    let invalid: LocalizedStringKey

    var body: some View {
        if let ok {
            Image(systemName: ok ? "checkmark.circle.fill" : "xmark.circle.fill")
                .foregroundStyle(ok ? Theme.inflow : Theme.negative)
                .help(ok ? valid : invalid)
                .accessibilityLabel(ok ? valid : invalid)
        }
    }
}

/// Where the recipient name field is, for the suggestion list over the form.
struct NameFieldBounds: PreferenceKey {
    static let defaultValue: Anchor<CGRect>? = nil
    static func reduce(value: inout Anchor<CGRect>?, nextValue: () -> Anchor<CGRect>?) {
        value = value ?? nextValue()
    }
}

/// The IBAN field of the transfer and standing order forms (#1475). It groups the IBAN in fours
/// while it is typed at the end, but never rewrites it while a character in the middle is
/// corrected: setting the whole text moves the cursor to the end. Then the grouping follows when
/// the field loses the focus.
struct IBANInput: View {
    @Binding var iban: String
    @FocusState private var focused: Bool

    /// Whether a change of the text is regrouped at once: any change from outside the field (a picked
    /// recipient, a template, a paste), or typing and deleting at the end. A correction in the middle
    /// is left as typed, so the cursor stays where the person put it.
    static func regroupsAtOnce(old: String, new: String, focused: Bool) -> Bool {
        guard focused else { return true }
        func clean(_ s: String) -> String { s.uppercased().filter { $0.isASCII && ($0.isLetter || $0.isNumber) } }
        let (o, n) = (clean(old), clean(new))
        return o != n && (n.hasPrefix(o) || o.hasPrefix(n))
    }

    private func regroup() {
        let f = TransferSheetView.groupedIBAN(iban)
        if f != iban { iban = f }
    }

    var body: some View {
        TextField("IBAN", text: $iban, prompt: Text("DE00 0000 0000 0000 0000 00"))
            .transferField()
            .appFont(.body, design: .monospaced)
            .autocorrectionDisabled()
            #if os(iOS)
            .textInputAutocapitalization(.characters)
            #endif
            .focused($focused)
            .onChange(of: iban) { old, new in
                if Self.regroupsAtOnce(old: old, new: new, focused: focused) { regroup() }
            }
            .onChange(of: focused) { _, now in if !now { regroup() } }
            .onSubmit { regroup() }
            .sensitive()
    }
}
