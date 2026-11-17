import XCTest
import GiroCrabCore
@testable import GiroCrabUI

@MainActor
final class AccountTotalsTooltipTests: XCTestCase {
    func testEyeTooltipNamesWinningGroupAndProtectsRedactedNames() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let target = try XCTUnwrap(model.allAccounts.first { $0.kind == .checking })
        let uuid = try XCTUnwrap(model.createUserGroup(name: "Tooltip-Testgruppe"))
        model.moveAccount(target.id, to: uuid)
        model.updateUserGroup(try XCTUnwrap(model.userGroups.first { $0.uuid == uuid }), includeInTotal: false)
        let account = try XCTUnwrap(model.allAccounts.first { $0.id == target.id })
        let help = model.totalExclusionIconHelp(account)
        XCTAssertTrue(help.contains("Tooltip-Testgruppe"), help)
        XCTAssertTrue(help.contains("trotz eingeschaltetem Kontoschalter"), help)
        let hidden = model.totalExclusionIconHelp(account, redaction: .labels)
        XCTAssertFalse(hidden.contains("Tooltip-Testgruppe"), hidden)
        XCTAssertTrue(hidden.contains("Eine Gruppe ist ausgeschlossen"), hidden)
        model.reportRedaction = .labels
        XCTAssertEqual(model.totalExclusionIconHelp(account), hidden)
    }

    func testEyeTooltipExplainsArchiveAndKeepsGenericOwnSwitchFallback() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let target = try XCTUnwrap(model.allAccounts.first { $0.kind == .order })
        XCTAssertFalse(target.includeInTotal)
        XCTAssertEqual(model.totalExclusionIconHelp(target), String(localized: "Nicht in der Gesamtsumme"))
        model.setAccountArchived(target, true)
        let archived = try XCTUnwrap(model.allAccounts.first { $0.id == target.id })
        XCTAssertTrue(model.totalExclusionIconHelp(archived).contains("archiviert"))
    }
}
