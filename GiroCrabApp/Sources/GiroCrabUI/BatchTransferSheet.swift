import SwiftUI
import UniformTypeIdentifiers
import GiroCrabCore

/// "Sammelüberweisung" (GitHub #240): many payments from one account in one
/// order (HKCCM) with one TAN. Table editor (resizable on macOS), CSV import,
/// then a read-only review. The bank's TAN request is shown by `TwoFactorSheet`.
struct BatchTransferSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.reportRedaction) private var redaction
    @Environment(\.dismiss) private var dismiss
    @Environment(\.textScale) private var textScale
    @Environment(\.undoManager) private var undoManager
    @Environment(\.dynamicTypeSize) private var typeSize
    /// Options and summary below each other: side by side they fit neither the iPhone nor the iPad's sheet.
    private var stacked: Bool {
        #if os(iOS)
        true
        #else
        false
        #endif
    }
    @State private var selection = Set<BatchRow.ID>()
    @State private var reviewing = false
    @State private var showImporter = false
    @State private var dropTargeted = false

    private var draft: BatchDraft { model.batchDraft }
    private var rows: [BatchRow] { draft.rows }
    private var invalid: Int { rows.filter { !$0.isValid }.count }
    private var totalCents: Int64 { rows.compactMap(\.cents).reduce(0, +) }
    private var account: Account? { model.batchAccounts.first { $0.id == draft.accountID } }
    private var pinStored: Bool {
        guard let id = account?.connectionId else { return false }
        return model.isDemo || (model.connectionStatus(id)?.hasPin ?? false)
    }
    private var canReview: Bool { account != nil && !rows.isEmpty && invalid == 0 }

    var body: some View {
        VStack(spacing: 0) {
            header
            if !reviewing { TransferKindPicker(batch: true, offersOwn: model.hasOwnTransferTargets).padding(.bottom, 10) }
            Divider()
            if reviewing { review } else { editor }
            if let e = model.transferError {
                Label(e, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(Theme.negative).appFont(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 20).padding(.vertical, 8)
            }
            Divider()
            buttons.padding(14)
        }
        #if os(macOS)
        // A table of many rows: the sheet can be resized (unlike the single transfer form).
        .frame(minWidth: 760 * textScale, idealWidth: 920 * textScale, maxWidth: .infinity,
               minHeight: 520 * textScale, idealHeight: 680 * textScale, maxHeight: .infinity)
        #endif
        .task { if model.options.demoSheet == "batch-transfer-confirm" { reviewing = true } } // screenshots
        .fileImporter(isPresented: $showImporter, allowedContentTypes: [.commaSeparatedText, .plainText, .text]) { result in
            if case .success(let url) = result { model.importBatchCSV(url) }
        }
        .overlay {
            if dropTargeted {
                RoundedRectangle(cornerRadius: 12).strokeBorder(Theme.accent, style: StrokeStyle(lineWidth: 2, dash: [6]))
                    .background(Theme.accent.opacity(0.06)).allowsHitTesting(false)
            }
        }
        .onDrop(of: [.fileURL], isTargeted: $dropTargeted) { providers in
            guard !reviewing, let p = providers.first else { return false }
            _ = p.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                Task { @MainActor in model.importBatchCSV(url) }
            }
            return true
        }
    }

    // MARK: header

    /// Title left, the import right; below the title where it does not fit beside it (iPhone, large text).
    private var header: some View {
        let title = Text(reviewing ? "Sammelüberweisung prüfen" : "Neue Überweisung").appFont(.title2, weight: .semibold)
        return ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) {
                title
                Spacer()
                importButton
            }
            VStack(alignment: .leading, spacing: 8) {
                title.fixedSize(horizontal: false, vertical: true)
                importButton
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 20).padding(.vertical, 14)
    }

    @ViewBuilder private var importButton: some View {
        if !reviewing {
            Button { showImporter = true } label: { Label("CSV importieren …", systemImage: "square.and.arrow.down") }
                .help("Spalten: Name, IBAN, Betrag, Verwendungszweck (Trennzeichen ; , oder Tab). Datei hierher ziehen.")
        }
    }

    // MARK: editor

    private var bookingText: String {
        draft.singleBookings ? String(localized: "Einzelbuchungen pro Zahlung") : String(localized: "Eine Sammelbuchung")
    }

    /// The choices side by side (label above) instead of a tall form; on iOS below each other.
    private var options: some View {
        @Bindable var draft = model.batchDraft
        let layout = stacked ? AnyLayout(VStackLayout(alignment: .leading, spacing: 12))
            : AnyLayout(HStackLayout(alignment: .top, spacing: 16))
        return VStack(alignment: .leading, spacing: 8) {
            layout {
                labeled("Von Konto", value: accountText, hiding: .personal) {
                    SourceAccountPicker(selection: $draft.accountID, accounts: model.batchAccounts, inForm: false)
                }
                labeled("Buchung auf dem Kontoauszug", value: bookingText) {
                    Picker("Buchung", selection: $draft.singleBookings) {
                        Text("Eine Sammelbuchung").tag(false)
                        Text("Einzelbuchungen pro Zahlung").tag(true)
                    }
                }
                .frame(width: stacked ? nil : 200 * textScale)
                labeled("Ausführung", value: draft.scheduled ? String(localized: "Terminüberweisung") : String(localized: "Normale Überweisung")) {
                    Picker("Ausführung", selection: $draft.scheduled) {
                        Text("Normale Überweisung").tag(false)
                        Text("Terminüberweisung").tag(true)
                    }
                }
                .frame(width: stacked ? nil : 150 * textScale)
                // Always there (disabled unless scheduled): the layout does not jump.
                labeled("Ausführungsdatum") {
                    DatePicker("Ausführungsdatum", selection: $draft.executionDate,
                               in: (Calendar.current.date(byAdding: .day, value: 1, to: Date()) ?? Date())...,
                               displayedComponents: .date)
                        .disabled(!draft.scheduled)
                }
                .fixedSize(horizontal: !stacked, vertical: true)
            }
            if let note = draft.importNote {
                Label(note, systemImage: "doc.text").appFont(.callout).foregroundStyle(Theme.secondary)
            }
        }
        #if os(macOS)
        .padding(.horizontal, 20).padding(.vertical, 12)
        #endif
    }

    private var accountText: String { account.map { "\($0.name) · \(formatIBAN($0.iban))" } ?? "–" }

    /// `value`: the choice as text. On iOS it is the label of a menu, which grows with it;
    /// a menu-style picker overlaps its neighbours when its text needs several lines.
    private func labeled<C: View>(_ title: LocalizedStringKey, value: String? = nil, hiding: ReportRedaction = [],
                                  @ViewBuilder _ content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).appFont(.caption).foregroundStyle(Theme.secondary)
            if stacked, let value {
                Menu { content().pickerStyle(.inline) } label: {
                    HStack(spacing: 6) {
                        Text(value).appFont(.body).multilineTextAlignment(.leading)
                            .fixedSize(horizontal: false, vertical: true)
                        Image(systemName: "chevron.up.chevron.down").appFont(.footnote).accessibilityHidden(true)
                    }
                }
                .accessibilityLabel(Text(title)).accessibilityValue(value)
                // On the whole menu: hidden, it neither opens nor reads out the accounts.
                .sensitive(hiding)
            } else {
                content().labelsHidden().pickerStyle(.menu).frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private var editor: some View {
        @Bindable var draft = model.batchDraft
        return VStack(spacing: 0) {
            #if os(macOS)
            options
            Divider()
            Table($draft.rows, selection: $selection) {
                TableColumn("Name") { $r in
                    TextField("Name", text: $r.name, prompt: Text("Name")).labelsHidden()
                        .sensitive()
                }
                .width(min: 120, ideal: 170)
                TableColumn("IBAN") { $r in
                    HStack(spacing: 6) {
                        TextField("IBAN", text: $r.iban, prompt: Text("DE00 …")).labelsHidden()
                            .appFont(.body, design: .monospaced)
                            .sensitive()
                        if !r.iban.isEmpty && !r.ibanValid {
                            Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.negative).help("IBAN ungültig (Prüfsumme/Länge)")
                                .accessibilityLabel("IBAN ungültig")
                        }
                    }
                }
                .width(min: 220 * textScale, ideal: 245 * textScale, max: 280 * textScale)
                TableColumn("Betrag") { $r in
                    HStack(spacing: 4) {
                        if r.cents == nil {
                            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Theme.negative).help("Betrag > 0 eingeben")
                                .accessibilityLabel("Betrag fehlt")
                        }
                        TextField("Betrag", text: $r.amountText, prompt: Text("0,00")).labelsHidden()
                            .multilineTextAlignment(.trailing).appFont(.body, monospacedDigit: true)
                            .sensitive()
                    }
                }
                .width(min: 80, ideal: 95, max: 120)
                TableColumn("Verwendungszweck") { $r in
                    TextField("Zweck", text: $r.purpose, prompt: Text("Verwendungszweck")).labelsHidden()
                        .sensitive()
                }
                .width(min: 160, ideal: 260)
            }
            .frame(maxHeight: .infinity)
            .onDeleteCommand { removeSelected() }
            #else
            // iPhone: a Table collapses to its first column, so a list of cards. The options
            // scroll with it: above it, they left no room for the cards.
            List {
                options
                ForEach($draft.rows) { $r in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            TextField("Name", text: $r.name, prompt: Text("Name"))
                            Spacer()
                            TextField("Betrag", text: $r.amountText, prompt: Text("0,00"))
                                .multilineTextAlignment(.trailing).monospacedDigit().fixedSize()
                        }
                        TextField("IBAN", text: $r.iban, prompt: Text("DE00 …"))
                            .appFont(.caption, design: .monospaced)
                            .foregroundStyle(r.ibanValid ? Theme.secondary : Theme.negative)
                        TextField("Zweck", text: $r.purpose, prompt: Text("Verwendungszweck"))
                            .appFont(.caption).foregroundStyle(Theme.secondary)
                    }
                    .sensitive()
                }
                .onDelete { offsets in draft.remove(Set(offsets.map { draft.rows[$0].id }), undo: undoManager) }
            }
            .listStyle(.plain)
            #endif

            Divider()
            HStack(spacing: 8) {
                Button { draft.rows.append(BatchRow()) } label: { Image(systemName: "plus") }
                    .help("Zeile hinzufügen").accessibilityLabel("Zeile hinzufügen")
                #if os(macOS)
                Button { removeSelected() } label: { Image(systemName: "minus") }
                    .help("Ausgewählte Zeilen entfernen (⌘Z macht es rückgängig)").disabled(selection.isEmpty)
                    .accessibilityLabel("Ausgewählte Zeilen entfernen")
                #endif
                Spacer()
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) { totals }
                    VStack(alignment: .trailing, spacing: 2) { totals }.multilineTextAlignment(.trailing)
                }
            }
            .padding(.horizontal, 20).padding(.vertical, 10)
        }
    }

    @ViewBuilder private var totals: some View {
        if invalid > 0 {
            Label("\(invalid) fehlerhaft", systemImage: "exclamationmark.triangle.fill")
                .appFont(.callout).foregroundStyle(Theme.negative)
        }
        Text("\(rows.count) Zahlungen · \(String(localized: "Summe"))").appFont(.callout).foregroundStyle(Theme.secondary)
        Text(Money.format(totalCents)).appFont(.title3, weight: .semibold, monospacedDigit: true).fixedSize().sensitive(.amounts)
    }

    // MARK: review

    /// One compact band: the total, then what and from where.
    private var summary: some View {
        let layout = stacked ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
            : AnyLayout(HStackLayout(alignment: .center, spacing: 20))
        return layout {
            VStack(alignment: .leading, spacing: 2) {
                Text("Summe").appFont(.caption).foregroundStyle(Theme.secondary)
                Text(Money.format(totalCents)).appFont(.title, weight: .semibold, monospacedDigit: true).fixedSize().sensitive(.amounts)
            }
            if !stacked { Divider().frame(height: 40) }
            VStack(alignment: .leading, spacing: 3) {
                (Text("\(rows.count) Zahlungen") + Text(" · ") + Text(account.map { "\(redaction.masking($0.name)) · \(redaction.masking(formatIBAN($0.iban)))" } ?? "")).lineLimit(stacked ? nil : 1)
                Text("\(bookingText) · \(draft.scheduled ? String(localized: "Terminüberweisung") : String(localized: "Heute (SEPA-Überweisung)"))")
                    .foregroundStyle(Theme.secondary).lineLimit(stacked ? nil : 1)
            }
            .appFont(.callout)
            if !stacked { Spacer(minLength: 0) }
        }
        #if os(macOS)
        .padding(.horizontal, 20).padding(.vertical, 12)
        #endif
    }

    private var review: some View {
        VStack(spacing: 0) {
            #if os(macOS)
            summary
            Divider()
            Table(rows) {
                TableColumn("Name") { r in Text(r.name).sensitive() }.width(min: 120, ideal: 170)
                TableColumn("IBAN") { r in Text(r.iban).appFont(.body, design: .monospaced).sensitive() }
                    .width(min: 220 * textScale, ideal: 245 * textScale, max: 280 * textScale)
                TableColumn("Betrag") { r in
                    Text(Money.format(r.cents ?? 0)).appFont(.body, monospacedDigit: true).sensitive(.amounts)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
                .width(min: 80, ideal: 95, max: 120)
                TableColumn("Verwendungszweck") { r in Text(r.purpose).sensitive() }.width(min: 160, ideal: 260)
            }
            .frame(maxHeight: .infinity)
            #else
            List {
                summary
                if typeSize.isAccessibilitySize { warning }
                ForEach(rows) { r in
                    VStack(alignment: .leading, spacing: 2) {
                        let amount = Text(Money.format(r.cents ?? 0)).monospacedDigit()
                        if typeSize.isAccessibilitySize {
                            // On its own line: at the largest sizes an amount can be wider than the row.
                            Text(r.name)
                            amount
                        } else {
                            HStack {
                                Text(r.name)
                                Spacer()
                                amount.fixedSize()
                            }
                        }
                        Text(r.iban).appFont(.caption, design: .monospaced).foregroundStyle(Theme.secondary)
                        Text(r.purpose).appFont(.caption).foregroundStyle(Theme.secondary)
                    }
                    .sensitive([.personal, .amounts])
                }
            }
            .listStyle(.plain)
            #endif
            Divider()
            pinArea
            // With the largest text it fills half the screen: there it scrolls with the list (above the payments).
            if !typeSize.isAccessibilitySize { warning.padding(.horizontal, 20).padding(.vertical, 10) }
        }
    }

    private var warning: some View {
        Label("Bitte Empfänger und IBAN genau prüfen. Eine freigegebene Überweisung lässt sich meist nicht mehr zurückholen.", systemImage: "exclamationmark.shield")
            .appFont(.callout).foregroundStyle(Theme.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private var pinArea: some View {
        @Bindable var draft = model.batchDraft
        if !pinStored {
            SecureField("Banking-PIN", text: $draft.pin, prompt: Text("wird nicht gespeichert"))
                .frame(maxWidth: 360 * textScale)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 20).padding(.top, 10)
        }
    }

    private func removeSelected() {
        draft.remove(selection, undo: undoManager)
        selection = []
    }

    @ViewBuilder private var buttons: some View {
        if typeSize.isAccessibilitySize {
            ViewThatFits(in: .horizontal) {
                HStack { buttonItems }
                VStack(spacing: 10) { buttonItems }.fixedSize(horizontal: false, vertical: true)
            }
        } else {
            HStack { buttonItems }
        }
    }

    @ViewBuilder private var buttonItems: some View {
        Button("Abbrechen", role: .cancel) { model.cancelBatchSheet(); dismiss() }.keyboardShortcut(.cancelAction)
        Spacer()
        if reviewing {
            Button("Zurück") { reviewing = false; model.transferError = nil }
            Button("Jetzt mit TAN freigeben") { model.startBatch() }
                .keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent)
        } else {
            Button("Weiter") { model.transferError = nil; reviewing = true }
                .keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent).disabled(!canReview)
        }
    }
}

/// "Überweisung | Umbuchung | Sammelüberweisung" under the title of both
/// transfer sheets (#881): one switch instead of two stacked ones.
struct TransferKindPicker: View {
    @Environment(AppModel.self) private var model
    let batch: Bool
    /// Umbuchung needs another own account with an IBAN.
    let offersOwn: Bool

    var body: some View {
        // Its own width when that fits; a narrow sheet (the closed iPhone Duo, whose camera
        // takes part of the width) squeezes the segments instead of widening the whole sheet.
        ViewThatFits(in: .horizontal) {
            picker.fixedSize()
            #if os(iOS)
            segments.padding(.horizontal, 20)
            #else
            picker.padding(.horizontal, 20)
            #endif
        }
        .frame(maxWidth: .infinity)
    }

    private var selection: Binding<TransferKind> {
        Binding(get: { batch ? .batch : (model.transferDraft.ownTransfer ? .own : .single) },
                set: { model.switchTransferKind(to: $0, fromBatch: batch) })
    }

    private var picker: some View {
        Picker("Art", selection: selection) {
            Text("Überweisung").tag(TransferKind.single)
            if offersOwn { Text("Umbuchung").tag(TransferKind.own) }
            if model.canBatchTransfer { Text("Sammelüberweisung").tag(TransferKind.batch) }
        }
        .pickerStyle(.segmented).labelsHidden()
    }

    #if os(iOS)
    private var segments: some View {
        var kinds: [(TransferKind, String)] = [(.single, String(localized: "Überweisung"))]
        if offersOwn { kinds.append((.own, String(localized: "Umbuchung"))) }
        if model.canBatchTransfer { kinds.append((.batch, String(localized: "Sammelüberweisung"))) }
        return ContentSizedSegments(titles: kinds.map(\.1), selection: Binding(
            get: { kinds.firstIndex { $0.0 == selection.wrappedValue } ?? 0 },
            set: { selection.wrappedValue = kinds[$0].0 }))
    }
    #endif
}

#if os(iOS)
/// The segmented picker with segments as wide as their titles need: in segments of equal
/// width the iPhone cut off "Sammelüberweisung".
private struct ContentSizedSegments: UIViewRepresentable {
    let titles: [String]
    @Binding var selection: Int

    func makeUIView(context: Context) -> UISegmentedControl {
        let control = UISegmentedControl()
        control.apportionsSegmentWidthsByContent = true
        control.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        control.addTarget(context.coordinator, action: #selector(Coordinator.changed), for: .valueChanged)
        return control
    }

    func updateUIView(_ control: UISegmentedControl, context: Context) {
        context.coordinator.selection = $selection
        if (0..<control.numberOfSegments).map({ control.titleForSegment(at: $0) ?? "" }) != titles {
            control.removeAllSegments()
            for (i, title) in titles.enumerated() { control.insertSegment(withTitle: title, at: i, animated: false) }
        }
        control.selectedSegmentIndex = selection
    }

    func makeCoordinator() -> Coordinator { Coordinator($selection) }

    @MainActor final class Coordinator: NSObject {
        var selection: Binding<Int>
        init(_ selection: Binding<Int>) { self.selection = selection }
        @objc func changed(_ control: UISegmentedControl) { selection.wrappedValue = control.selectedSegmentIndex }
    }
}
#endif
