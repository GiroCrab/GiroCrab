import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Rules and tags to and from a file (#1057), through the app model.
@MainActor
final class RuleTagExchangeTests: XCTestCase {
    private func tempFile() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("rule-exchange-\(UUID().uuidString).json")
    }

    func testRoundTripInTheSameVaultAddsNothing() async throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        model.reloadRules()
        if model.rules.isEmpty {
            let category = try XCTUnwrap(model.categories.first).id
            _ = try await model.requireVaultOffMain {
                try $0.saveRule(rule: Rule(id: 0, name: "Testregel", categoryId: category, field: .counterparty,
                                           matchKind: .keyword, pattern: "Testladen", caseSensitive: false,
                                           minAmountCents: nil, maxAmountCents: nil, accountIds: [], priority: 0,
                                           enabled: true, matchCount: 0, tagIds: []))
            }
            model.reloadRules()
        }
        let count = model.rules.count
        XCTAssertGreaterThan(count, 0)

        let exported = await model.exportRulesFile([])
        let data = try XCTUnwrap(exported)
        let json = try XCTUnwrap(String(data: data, encoding: .utf8))
        XCTAssertTrue(json.contains("\"girocrab-rules-tags\""), json)
        let url = tempFile()
        try data.write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }

        // the preview lists every rule, each one already present here
        let previewed = await model.previewRulesTagsFile(url)
        let file = try XCTUnwrap(previewed)
        XCTAssertEqual(file.preview.rules.count, count)
        XCTAssertTrue(file.preview.rules.allSatisfy { $0.problem != nil }, "\(file.preview.rules)")
        XCTAssertTrue(file.preview.tags.allSatisfy(\.exists))

        let imported = await model.importRulesAndTags(file, rules: file.preview.rules.map(\.index),
                                                      tags: file.preview.tags.map(\.name))
        let report = try XCTUnwrap(imported)
        XCTAssertEqual(report.rulesImported, 0)
        XCTAssertEqual(report.rulesSkipped, UInt32(count))
        XCTAssertEqual(report.tagsCreated, 0)
        XCTAssertEqual(model.rules.count, count)
    }

    func testForeignFileShowsAnError() async throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let url = tempFile()
        try Data(#"{"hello":"world"}"#.utf8).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        let preview = await model.previewRulesTagsFile(url)
        XCTAssertNil(preview)
    }

    /// A ticked rule keeps its tags ticked (case-insensitive); unticked rules don't.
    func testTagsOfTickedRulesAreRequired() {
        let preview = RuleExchangePreview(
            rules: [
                RulePreview(index: 0, name: "rewe", categoryPath: "Lebensmittel", tags: ["Familie"], problem: nil, note: nil),
                RulePreview(index: 1, name: "kino", categoryPath: "Freizeit", tags: ["Urlaub"], problem: nil, note: nil),
            ],
            tags: [TagPreview(name: "Familie", colorHex: "", exists: false), TagPreview(name: "Urlaub", colorHex: "", exists: true)]
        )
        XCTAssertEqual(RuleExchangeSelectionSheet.requiredTags(preview, rules: [0]), ["familie"])
        XCTAssertEqual(RuleExchangeSelectionSheet.requiredTags(preview, rules: [0, 1]), ["familie", "urlaub"])
        XCTAssertTrue(RuleExchangeSelectionSheet.requiredTags(preview, rules: []).isEmpty)
        // a rule the import skips locks no tag
        var skipped = preview
        skipped.rules[1].problem = "bereits vorhanden"
        XCTAssertEqual(RuleExchangeSelectionSheet.requiredTags(skipped, rules: [0, 1]), ["familie"])
    }

    func testRulesFileSummaryAndDetails() {
        let report = RuleExchangeReport(tagsCreated: 2, tagsExisting: 1, rulesImported: 3, rulesSkipped: 1,
                                        rulesImportedLines: ["rewe", "hotel", "„beide“ – ohne die nicht gefundenen Konten (Karte)"],
                                        rulesSkippedLines: ["„kino“: Kategorie „Freizeit“ nicht gefunden"],
                                        tagsCreatedNames: ["Urlaub", "Auto"], tagsExistingNames: ["Familie"])
        let lines = RuleExchangeReportSheet.summaryLines(report)
        XCTAssertTrue(lines[0].contains("3") && lines[0].contains("1"), "\(lines)")
        XCTAssertTrue(lines[1].contains("2"), "\(lines)")
        // the counts only; names and reasons are in the details
        XCTAssertFalse(lines.joined().contains("kino"))
        let sections = RuleExchangeReportSheet.detailSections(report)
        // imported and skipped, for rules and for tags
        XCTAssertEqual(sections.map(\.lines),
                       [report.rulesImportedLines, report.rulesSkippedLines, ["Urlaub", "Auto"], ["Familie"]])
    }

    /// A tags file: no rule line, but the tags in the details as for rules.
    func testTagsFileHasDetailsToo() {
        let report = RuleExchangeReport(tagsCreated: 1, tagsExisting: 2, rulesImported: 0, rulesSkipped: 0,
                                        rulesImportedLines: [], rulesSkippedLines: [],
                                        tagsCreatedNames: ["Urlaub"], tagsExistingNames: ["Auto", "Abos"])
        let lines = RuleExchangeReportSheet.summaryLines(report)
        XCTAssertEqual(lines.count, 1, "\(lines)")
        XCTAssertEqual(RuleExchangeReportSheet.detailSections(report).map(\.lines), [["Urlaub"], ["Auto", "Abos"]])
    }
}
