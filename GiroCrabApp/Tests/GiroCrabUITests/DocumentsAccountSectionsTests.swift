import Foundation
import Testing
@testable import GiroCrabUI

// test: serial-defaults
@Suite @MainActor
struct DocumentsAccountSectionsTests {
    private func demoModel() -> AppModel {
        AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
    }

    private func listedIDs(_ model: AppModel) -> Set<Int64> {
        Set(model.documentsAccountSections.flatMap(\.accounts).map(\.id))
    }

    @Test func archivedAccountIsNotOffered() throws {
        let model = demoModel()
        let account = try #require(model.accountMenuSections.first?.accounts.first)
        #expect(listedIDs(model).contains(account.id))

        model.setAccountArchived(account, true)

        #expect(!listedIDs(model).contains(account.id))
        // Regel-Picker und „Konto zuordnen“ nutzen weiter die ungefilterte Liste.
        #expect(model.accountMenuSections.flatMap(\.accounts).map(\.id).contains(account.id))
    }

    @Test func archivedAccountStaysVisibleWhileSelected() throws {
        let model = demoModel()
        let account = try #require(model.accountMenuSections.first?.accounts.first)
        model.setAccountArchived(account, true)
        model.documentsAccountFilter = account.id
        #expect(listedIDs(model).contains(account.id))
    }
}
