import SwiftUI
import UniformTypeIdentifiers
import GiroCrabCore

/// JSON handed to `.fileExporter`.
struct JSONFileDocument: FileDocument {
    static var readableContentTypes: [UTType] { [.json] }
    var data: Data
    init(data: Data) { self.data = data }
    init(configuration: ReadConfiguration) throws { data = configuration.file.regularFileContents ?? Data() }
    func fileWrapper(configuration: WriteConfiguration) -> FileWrapper { FileWrapper(regularFileWithContents: data) }
}

/// What the rule window and the tag manager export (#1057).
enum RuleTagExport: Equatable {
    case rules([Int64])
    case tags([Int64])

    var defaultFilename: String {
        switch self {
        case .rules(let ids): ids.count == 1 ? String(localized: "GiroCrab-Regel") : String(localized: "GiroCrab-Regeln")
        case .tags(let ids): ids.count == 1 ? String(localized: "GiroCrab-Tag") : String(localized: "GiroCrab-Tags")
        }
    }
}

/// A read rules/tags file waiting for the user's choice (#1057).
struct RuleTagImport: Identifiable {
    let id = UUID()
    let json: String
    let preview: RuleExchangePreview
}

/// Export and import of rules and tags as a JSON file (#1057): the save and open
/// panels, the choice of what to import and the summary after it. Shared by
/// `RuleEditorView` and `TagManagerView`; set `export` or `importing` to start.
struct RuleTagExchange: ViewModifier {
    @Environment(AppModel.self) private var model
    @Binding var export: RuleTagExport?
    @Binding var importing: Bool
    @State private var document: JSONFileDocument?
    @State private var filename = ""
    /// The file read, while the user chooses what to import.
    @State private var pending: RuleTagImport?
    /// What was chosen; imported once the selection sheet is gone (one sheet at a time).
    @State private var chosen: (file: RuleTagImport, rules: [UInt32], tags: [String])?
    @State private var report: RuleExchangeReport?

    func body(content: Content) -> some View {
        content
            .task(id: export) {
                guard let export else { return }
                let data: Data? = switch export {
                case .rules(let ids): await model.exportRulesFile(ids)
                case .tags(let ids): await model.exportTagsFile(ids)
                }
                filename = export.defaultFilename
                document = data.map(JSONFileDocument.init(data:))
                if document == nil { self.export = nil }
            }
            .fileExporter(
                isPresented: Binding(get: { document != nil }, set: { if !$0 { document = nil; export = nil } }),
                document: document,
                contentType: .json,
                defaultFilename: filename
            ) { result in
                if case .failure(let e) = result {
                    model.showError(String(localized: "Export fehlgeschlagen"), e.localizedDescription)
                }
            }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.json]) { result in
                switch result {
                case .success(let url):
                    Task { pending = await model.previewRulesTagsFile(url) }
                case .failure(let e):
                    model.showError(String(localized: "Import fehlgeschlagen"), e.localizedDescription)
                }
            }
            .capturableSheet(item: $pending, onDismiss: runChosenImport) { file in
                RuleExchangeSelectionSheet(preview: file.preview) { rules, tags in
                    chosen = (file, rules, tags)
                }
            }
            .capturableSheet(isPresented: Binding(get: { report != nil }, set: { if !$0 { report = nil } })) {
                if let report { RuleExchangeReportSheet(report: report) }
            }
    }

    private func runChosenImport() {
        guard let chosen else { return }
        self.chosen = nil
        Task { report = await model.importRulesAndTags(chosen.file, rules: chosen.rules, tags: chosen.tags) }
    }
}

/// Before an import (#1057): every rule and tag of the file, all ticked; untick
/// what should stay out. Each row says what the import would do with it. The
/// tags a ticked rule adds always come along (ticked and locked).
struct RuleExchangeSelectionSheet: View {
    let preview: RuleExchangePreview
    /// Chosen rule positions and tag names; called before the sheet closes.
    let onImport: ([UInt32], [String]) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.textScale) private var textScale
    @State private var rules: Set<UInt32>
    @State private var tags: Set<String>

    init(preview: RuleExchangePreview, onImport: @escaping ([UInt32], [String]) -> Void) {
        self.preview = preview
        self.onImport = onImport
        _rules = State(initialValue: Set(preview.rules.map(\.index)))
        _tags = State(initialValue: Set(preview.tags.map(\.name)))
    }

    /// Tags a ticked rule that will be imported adds, as the core's name keys
    /// (`AppModel.tagKey`: case and inner spaces ignored): imported in any case.
    /// A rule the import skips brings no tag along.
    static func requiredTags(_ preview: RuleExchangePreview, rules: Set<UInt32>) -> Set<String> {
        Set(preview.rules.filter { rules.contains($0.index) && $0.problem == nil }.flatMap(\.tags).map(AppModel.tagKey))
    }

    private var required: Set<String> { Self.requiredTags(preview, rules: rules) }

    private func isLocked(_ tag: TagPreview) -> Bool { required.contains(AppModel.tagKey(tag.name)) }

    private func isTicked(_ tag: TagPreview) -> Bool { tags.contains(tag.name) || isLocked(tag) }

    private var tagCount: Int { preview.tags.filter(isTicked).count }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Import auswählen").appFont(.headline)
            Text("Alles ist ausgewählt. Was nicht importiert werden soll, abwählen.")
                .appFont(.callout).foregroundStyle(Theme.secondary)
                .fixedSize(horizontal: false, vertical: true)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if !preview.rules.isEmpty { rulesSection }
                    if !preview.tags.isEmpty { tagsSection }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 380 * textScale)
            HStack {
                Spacer()
                Button("Abbrechen", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(String(localized: "Importieren (\(rules.count + tagCount))")) {
                    onImport(rules.sorted(), preview.tags.filter(isTicked).map(\.name))
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(rules.isEmpty && tagCount == 0)
            }
        }
        .padding(20)
        #if os(macOS)
        .frame(width: 520 * textScale)
        #endif
    }

    private func header(_ title: String, all: @escaping () -> Void, none: @escaping () -> Void) -> some View {
        HStack {
            Text(title).appFont(.subheadline, weight: .semibold)
            Spacer()
            Button("Alle", action: all)
            Button("Keine", action: none)
        }
        .buttonStyle(.borderless)
    }

    private var rulesSection: some View {
        VStack(alignment: .leading, spacing: 6) {
            header(String(localized: "Regeln (\(rules.count) von \(preview.rules.count))"),
                   all: { rules = Set(preview.rules.map(\.index)) },
                   none: { rules = [] })
            ForEach(preview.rules, id: \.index) { rule in
                Toggle(isOn: Binding(
                    get: { rules.contains(rule.index) },
                    set: { if $0 { rules.insert(rule.index) } else { rules.remove(rule.index) } }
                )) {
                    VStack(alignment: .leading, spacing: 1) {
                        // rule names are mostly payees
                        Text(rule.name).appFont(.body).sensitive()
                        if !rule.categoryPath.isEmpty {
                            Text("→ " + rule.categoryPath).appFont(.caption).foregroundStyle(Theme.secondary)
                                .sensitive(.labels)
                        }
                        if !rule.tags.isEmpty {
                            Text(rule.tags.map { "#" + $0 }.joined(separator: " ")).appFont(.caption)
                                .foregroundStyle(Theme.secondary).sensitive()
                        }
                        if let problem = rule.problem {
                            Text(String(localized: "Wird übersprungen: \(problem)")).appFont(.caption)
                                .foregroundStyle(Theme.warning).sensitive()
                        } else if let note = rule.note {
                            Text(note).appFont(.caption).foregroundStyle(Theme.warning).sensitive()
                        }
                    }
                    .fixedSize(horizontal: false, vertical: true)
                }
                #if os(macOS)
                .toggleStyle(.checkbox)
                #endif
            }
        }
    }

    private var tagsSection: some View {
        VStack(alignment: .leading, spacing: 6) {
            header(String(localized: "Tags (\(tagCount) von \(preview.tags.count))"),
                   all: { tags = Set(preview.tags.map(\.name)) },
                   none: { tags = [] })
            ForEach(preview.tags, id: \.name) { tag in
                let locked = isLocked(tag)
                Toggle(isOn: Binding(
                    get: { isTicked(tag) },
                    set: { if $0 { tags.insert(tag.name) } else { tags.remove(tag.name) } }
                )) {
                    VStack(alignment: .leading, spacing: 1) {
                        // tag names are always covered in a report (#228)
                        Text("#" + tag.name).appFont(.body).sensitive()
                        if let problem = tag.problem {
                            Text(String(localized: "Wird übersprungen: \(problem)")).appFont(.caption)
                                .foregroundStyle(Theme.warning)
                        } else {
                            Text(locked ? String(localized: "von einer ausgewählten Regel verwendet")
                                 : tag.exists ? String(localized: "bereits vorhanden, wird übernommen")
                                 : String(localized: "neu"))
                                .appFont(.caption).foregroundStyle(Theme.secondary)
                        }
                    }
                }
                #if os(macOS)
                .toggleStyle(.checkbox)
                #endif
                .disabled(locked)
            }
        }
    }
}

/// Summary after an import (#1057): the counts, and below them „Details“ (folded by
/// default) with the skipped rules and the tags created or found, for a rules file
/// as for a tags file.
struct RuleExchangeReportSheet: View {
    let report: RuleExchangeReport
    @Environment(\.dismiss) private var dismiss
    @Environment(\.textScale) private var textScale
    @State private var showDetails = false

    /// A titled block of the details; `lines` hold personal names.
    struct Section: Identifiable {
        let title: String
        let lines: [String]
        var id: String { title }
    }

    /// Count lines: rules only when the file had rules, tags only when it had tags.
    static func summaryLines(_ r: RuleExchangeReport) -> [String] {
        var lines: [String] = []
        if r.rulesImported + r.rulesSkipped > 0 {
            lines.append(String(localized: "Regeln importiert: \(r.rulesImported) · übersprungen: \(r.rulesSkipped)"))
        }
        if r.tagsCreated + r.tagsExisting > 0 || !r.tagsSkippedLines.isEmpty {
            lines.append(String(localized: "Tags importiert: \(r.tagsCreated) · übersprungen: \(r.tagsExisting + UInt32(r.tagsSkippedLines.count))"))
        }
        if lines.isEmpty {
            lines.append(String(localized: "Die Datei enthält keine Regeln oder Tags."))
        }
        if r.rulesImported > 0 {
            lines.append(String(localized: "Bestehende Buchungen ändern sich erst mit „Alle Regeln anwenden“."))
        }
        return lines
    }

    /// What „Details“ shows, imported before skipped, rules before tags; empty
    /// sections are left out, no section at all = no „Details“.
    static func detailSections(_ r: RuleExchangeReport) -> [Section] {
        [
            Section(title: String(localized: "Importierte Regeln"), lines: r.rulesImportedLines),
            Section(title: String(localized: "Übersprungene Regeln"), lines: r.rulesSkippedLines),
            Section(title: String(localized: "Importierte Tags"), lines: r.tagsCreatedNames),
            // already there (reused by the rules) or not creatable (invalid name)
            Section(title: String(localized: "Übersprungene Tags"),
                    lines: r.tagsExistingNames.map { String(localized: "„\($0)“: bereits vorhanden") } + r.tagsSkippedLines),
        ].filter { !$0.lines.isEmpty }
    }

    var body: some View {
        let sections = Self.detailSections(report)
        VStack(alignment: .leading, spacing: 12) {
            Text("Import abgeschlossen").appFont(.headline)
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Self.summaryLines(report), id: \.self) { line in
                    Text(line).fixedSize(horizontal: false, vertical: true)
                }
            }
            .appFont(.body)
            if !sections.isEmpty {
                DisclosureGroup(isExpanded: $showDetails) {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(sections) { section in
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(section.title).appFont(.caption, weight: .semibold).foregroundStyle(Theme.secondary)
                                    ForEach(Array(section.lines.enumerated()), id: \.offset) { _, line in
                                        // rule, account and tag names
                                        Text(line).appFont(.callout).sensitive()
                                            .fixedSize(horizontal: false, vertical: true)
                                    }
                                }
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.top, 6)
                    }
                    .frame(maxHeight: 240 * textScale)
                } label: {
                    Text("Details").appFont(.body, weight: .semibold)
                }
            }
            HStack {
                Spacer()
                Button("OK") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        #if os(macOS)
        .frame(width: 460 * textScale)
        #endif
    }
}

extension View {
    /// Save and open panels for a rules/tags file (#1057).
    func ruleTagExchange(export: Binding<RuleTagExport?>, importing: Binding<Bool>) -> some View {
        modifier(RuleTagExchange(export: export, importing: importing))
    }
}
