import Foundation
import GiroCrabCore

extension AppModel {
    // MARK: - Rules and tags as a file (#1057)

    /// JSON of the rules `ids` (empty = all) with the tags they add; nil = locked or failed (error shown).
    func exportRulesFile(_ ids: [Int64]) async -> Data? {
        await exchangeExport { try $0.exportRules(ruleIds: ids) }
    }

    /// JSON of the tags `ids` (empty = all); nil = locked or failed (error shown).
    func exportTagsFile(_ ids: [Int64]) async -> Data? {
        await exchangeExport { try $0.exportTags(tagIds: ids) }
    }

    private func exchangeExport(_ work: @escaping @Sendable (Vault) throws -> String) async -> Data? {
        do {
            guard let json = try await vaultOffMain(work) else { return nil }
            return Data(json.utf8)
        } catch {
            showError(String(localized: "Export fehlgeschlagen"), describe(error))
            return nil
        }
    }

    /// Reads a rules/tags file chosen by the user and lists what it holds, for
    /// choosing what to import. nil = unreadable or no GiroCrab file (error shown).
    func previewRulesTagsFile(_ url: URL) async -> RuleTagImport? {
        let json: String
        do {
            // off the main thread; the security scope lasts until the read is done
            json = try await Task.detached(priority: .userInitiated) {
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                return try String(contentsOf: url, encoding: .utf8)
            }.value
        } catch {
            showError(String(localized: "Import fehlgeschlagen"), error.localizedDescription)
            return nil
        }
        do {
            guard let preview = try await vaultOffMain({ try $0.previewRulesAndTags(json: json) }) else { return nil }
            return RuleTagImport(json: json, preview: preview)
        } catch {
            showError(String(localized: "Import fehlgeschlagen"), describe(error))
            return nil
        }
    }

    /// Imports the chosen rules (file positions) and tags of a previewed file and
    /// reloads rules and tags. nil = failed (error shown).
    func importRulesAndTags(_ file: RuleTagImport, rules: [UInt32], tags: [String]) async -> RuleExchangeReport? {
        let json = file.json
        do {
            guard let report = try await vaultOffMain({
                try $0.importRulesAndTagsSelected(json: json, ruleIndices: rules, tagNames: tags)
            }) else { return nil }
            reloadTags()
            reloadRules()
            return report
        } catch {
            // tags created before the failure are in the vault: show them
            reloadTags()
            reloadRules()
            showError(String(localized: "Import fehlgeschlagen"), describe(error))
            return nil
        }
    }
}
