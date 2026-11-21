import Foundation
import GiroCrabCore

extension AppModel {
    // MARK: - Händlererkennung (#1373, Einstellungen › Händler)

    /// Master switch, the bundled merchants with their switches and the own merchants. Off the
    /// main thread: every bundled merchant comes with its signet. Nil when the vault was locked
    /// or switched meanwhile.
    func merchantRecognition() async throws -> MerchantRecognition? {
        try await vaultOffMain { try $0.merchantRecognition() }
    }

    /// Switches the whole recognition on or off. Throws the core's message for the settings
    /// pane to show (an alert of the main window would hide behind the Settings window).
    func setMerchantRecognitionEnabled(_ enabled: Bool) async throws {
        try await changeMerchants { try $0.setMerchantRecognitionEnabled(enabled: enabled) }
    }

    /// Switches a bundled merchant on or off; throws like `setMerchantRecognitionEnabled`.
    func setBuiltinMerchantEnabled(_ key: String, _ enabled: Bool) async throws {
        try await changeMerchants { try $0.setBuiltinMerchantEnabled(key: key, enabled: enabled) }
    }

    /// Creates (`uuid` nil) or changes an own merchant. `basedOn` (creating only) copies a
    /// bundled merchant, which the copy then replaces while it lives. Throws the core's message
    /// (no name, no search word …) for the editor to show next to its fields.
    func saveCustomMerchant(uuid: String?, name: String, needles: [String], basedOn: String?) async throws {
        try await changeMerchants {
            _ = try $0.saveCustomMerchant(uuid: uuid, name: name, needles: needles, basedOn: basedOn)
        }
    }

    /// Deletes an own merchant; once no copy of a bundled merchant is left, that one is
    /// recognised again (unless switched off).
    func deleteCustomMerchant(_ uuid: String) async throws {
        try await changeMerchants { try $0.deleteCustomMerchant(uuid: uuid) }
    }

    /// The search words of the editor's text field: separated by commas or line breaks (the
    /// core treats a comma inside a word as a space anyway), blanks dropped.
    nonisolated static func merchantNeedles(_ text: String) -> [String] {
        text.split(whereSeparator: { $0 == "," || $0 == ";" || $0.isNewline })
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
    }

    /// The editor's text for stored search words.
    nonisolated static func merchantNeedlesText(_ needles: [String]) -> String {
        needles.joined(separator: ", ")
    }

    /// One change of the recognition: queued behind earlier writes of the Settings window
    /// (`settingsWriteTask`) so quick toggles reach the vault in order, and run off the main
    /// thread (a list load may hold the vault). Only the last of several queued changes
    /// reloads (`merchantWriteCount`); a failed last one reloads too, for the ones before it.
    private func changeMerchants(_ change: @escaping @Sendable (Vault) throws -> Void) async throws {
        guard let vault else { throw CoreError.Database(message: "vault locked") }
        merchantWriteCount += 1
        let number = merchantWriteCount
        let previous = settingsWriteTask
        let write = Task { @MainActor () async throws -> Bool in
            await previous?.value
            return try await Self.offMain {
                try change(vault)
                return try vault.merchantRecognitionEnabled()
            }
        }
        settingsWriteTask = Task { _ = try? await write.value }
        let enabled: Bool
        do {
            enabled = try await write.value
        } catch {
            if number == merchantWriteCount, self.vault === vault { merchantsChanged() }
            throw error
        }
        // a lock or a switch to the demo vault meanwhile: nothing of the old vault to show
        guard self.vault === vault else { return }
        merchantRecognitionEnabled = enabled
        guard number == merchantWriteCount else { return }
        merchantsChanged()
    }

    /// Everything that shows merchant names, logos or a merchant's history loads again in the
    /// background; the shown data stays until then. Flows, totals, recurring series and the
    /// sidebar read the bundled table and stay as they are. A merchant filter is left: its key
    /// may name nobody any more (a copy has its own key, the recognition off gives `x:` keys).
    private func merchantsChanged() {
        merchantSettingsRevision += 1
        if merchantFocus != nil { leaveMerchantFocus() }
        refreshSelectedBookingHistory()
        startSelectionLoad(keepingShown: true)
        reloadDashboard()
    }
}
