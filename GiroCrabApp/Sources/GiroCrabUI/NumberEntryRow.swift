import SwiftUI

/// A whole number that can be typed, with a pop-up of the values the bank offers or a stepper
/// (standing order: "Alle", "Ausführungstag", #1475). A typed number counts at once when it is
/// allowed; when the field is left, a number the bank does not allow moves to the nearest one that it does.
struct NumberEntryRow: View {
    let title: LocalizedStringKey
    @Binding var value: Int
    let range: ClosedRange<Int>
    /// The values the bank allows (empty = every one in `range`).
    var allowed: [Int] = []
    /// Entries of the pop-up; empty = a stepper.
    var choices: [(value: Int, text: String)] = []
    /// Text beside the field for the current value.
    var note: (Int) -> String? = { _ in nil }
    /// Every text `note` can show: the field reserves the width of the longest, so it does not move
    /// when the text changes ("Monat" → "Monate").
    var noteSamples: [String] = []
    /// Whether the text stands before or after the field.
    var notePlacement: NotePlacement = .after

    enum NotePlacement { case before, after }

    @Environment(\.textScale) private var textScale
    @State private var text = ""
    @FocusState private var focused: Bool
    @State private var choicesOpen = false

    /// Rows of the pop-up shown at once; more scroll.
    private static let visibleChoices = 15

    private func accepts(_ n: Int) -> Bool { range.contains(n) && (allowed.isEmpty || allowed.contains(n)) }

    /// `n` moved into the range and, when the bank limits the values, onto the nearest allowed one.
    static func nearest(_ n: Int, range: ClosedRange<Int>, allowed: [Int]) -> Int {
        let c = min(max(n, range.lowerBound), range.upperBound)
        guard !allowed.isEmpty, !allowed.contains(c) else { return c }
        return allowed.min { abs($0 - c) < abs($1 - c) } ?? c
    }

    /// One step up or down: to the next value the bank allows, else by 1, never out of the range.
    static func step(_ n: Int, by d: Int, range: ClosedRange<Int>, allowed: [Int]) -> Int {
        guard !allowed.isEmpty else {
            if range.contains(n) { return min(max(n + d, range.lowerBound), range.upperBound) }
            // a value the bank holds outside the range (a day 99) is not moved into it by going further out;
            // going back moves to the nearest end of the range
            return n > range.upperBound ? (d > 0 ? n : range.upperBound) : (d < 0 ? n : range.lowerBound)
        }
        let inRange = allowed.filter { range.contains($0) }.sorted()
        let next = d > 0 ? inRange.first { $0 > n } : inRange.last { $0 < n }
        return next ?? (inRange.isEmpty ? n : (d > 0 ? inRange.last! : inRange.first!))
    }

    /// Where a step starts: the typed number when it lies in the range, else the current value
    /// (a day the bank holds above the range is not moved into it by a press).
    static func stepStart(typed: Int?, value: Int, range: ClosedRange<Int>) -> Int {
        guard let typed, range.contains(typed) else { return value }
        return typed
    }

    private func step(_ d: Int) {
        // a number typed but not yet taken over counts as the start; it is stepped from as typed, not snapped first
        let start = Self.stepStart(typed: Int(text), value: value, range: range)
        value = Self.step(start, by: d, range: range, allowed: allowed)
        text = String(value)
    }

    /// The value after the field was left: what was typed moved into the range and onto an allowed value;
    /// an unchanged text keeps the value, also one the bank holds outside the range.
    static func committed(text: String, value: Int, range: ClosedRange<Int>, allowed: [Int]) -> Int {
        guard let n = Int(text), n != value else { return value }
        return nearest(n, range: range, allowed: allowed)
    }

    private func commit() {
        value = Self.committed(text: text, value: value, range: range, allowed: allowed)
        text = String(value)
    }

    /// The text beside the field: the unit ("Monate") or the meaning of the value ("Letzter Tag des Monats"),
    /// without the number. It reserves the width of the longest sample.
    @ViewBuilder private var noteView: some View {
        if !noteSamples.isEmpty || note(value) != nil {
            ZStack(alignment: notePlacement == .before ? .trailing : .leading) {
                ForEach(noteSamples, id: \.self) {
                    Text($0).appFont(.callout).lineLimit(1).hidden().accessibilityHidden(true)
                }
                if let n = note(value) {
                    Text(n).appFont(.callout).foregroundStyle(Theme.secondary).lineLimit(1)
                }
            }
        }
    }

    /// The values to pick from: at most `visibleChoices` rows high, then it scrolls (to the current value).
    private var choicesList: some View {
        let rowHeight = 24 * textScale
        return ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(choices, id: \.value) { c in
                        Button { value = c.value; choicesOpen = false } label: {
                            HStack(spacing: 8) {
                                Image(systemName: "checkmark").opacity(c.value == value ? 1 : 0).frame(width: 14)
                                Text(c.text).appFont(.body)
                                Spacer(minLength: 0)
                            }
                            .padding(.horizontal, 10)
                            .frame(height: rowHeight)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .id(c.value)
                    }
                }
                .padding(.vertical, 4)
            }
            .frame(minWidth: 150 * textScale, maxHeight: CGFloat(min(choices.count, Self.visibleChoices)) * rowHeight + 8)
            .onAppear { proxy.scrollTo(value, anchor: .center) }
        }
    }

    var body: some View {
        LabeledContent(title) {
            HStack(spacing: 4) {
                if notePlacement == .before { noteView }
                TextField(title, text: $text)
                    .transferField(alignment: .trailing)
                    .appFont(.body, monospacedDigit: true)
                    .frame(width: 40 * textScale)  // two digits
                    .focused($focused)
                    .onSubmit(commit)
                    .onKeyPress(.upArrow) { step(1); return .handled }
                    .onKeyPress(.downArrow) { step(-1); return .handled }
                    #if os(iOS)
                    .keyboardType(.numberPad)
                    #endif
                if notePlacement == .after { noteView }
                // +1 / -1, then the list of the values (when there is one)
                Stepper(title, onIncrement: { step(1) }, onDecrement: { step(-1) }).labelsHidden()
                if !choices.isEmpty {
                    Button { choicesOpen = true } label: {
                        Image(systemName: "chevron.down").foregroundStyle(Theme.secondary)
                    }
                    .buttonStyle(.plain)
                    .help(title)
                    .accessibilityLabel(title)
                    .capturablePopover(isPresented: $choicesOpen, arrowEdge: .bottom) { choicesList }
                }
            }
        }
        .onAppear { text = String(value) }
        .onChange(of: text) { _, new in
            let digits = String(new.filter(\.isNumber).prefix(2))
            if digits != new { text = digits; return }
            if let n = Int(digits), accepts(n) { value = n }
        }
        .onChange(of: value) { _, new in if Int(text) != new { text = String(new) } }
        .onChange(of: focused) { _, now in if !now { commit() } }
    }
}
