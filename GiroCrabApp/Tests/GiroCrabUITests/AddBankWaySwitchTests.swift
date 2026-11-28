import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// "Bank hinzufügen": switching between the ways to one bank (direct, PSD2)
/// must not keep the last form's add action, and connectors that sign in with
/// the login name ask for it.
final class AddBankWaySwitchTests: XCTestCase {
    func testResetDropsTheLastFormsAction() {
        let submit = AddBankSubmit()
        var added = ""
        submit.update(true) { added = "psd2" }
        XCTAssertTrue(submit.canSubmit)
        submit.reset()
        XCTAssertFalse(submit.canSubmit)
        submit.action()
        XCTAssertEqual(added, "", "the PSD2 form's action is gone after switching away")
    }

    func testConnectorsThatSignInWithTheLoginRequireIt() {
        for kind: ConnectorKind in [.hanseaticBank, .easybank, .ibkr, .kontist, .volkswagenBank, .uberspace, .fordMoney, .opelDirektbank, .bhw, .comdirect] {
            XCTAssertTrue(KindAddForm.loginRequired(kind), "\(kind)")
        }
        // the field is only a display name there
        for kind: ConnectorKind in [.payPal, .bunq, .americanExpress, .amazonVisa, .bitcoinCore] {
            XCTAssertFalse(KindAddForm.loginRequired(kind), "\(kind)")
        }
    }

    func testFinTsBankNameFollowsTheBlz() {
        let db = "Deutsche Bank", pb = "Postbank Ndl der Deutsche Bank"
        // a BLZ missing from the directory drops the name the directory filled in
        XCTAssertEqual(FinTsAddForm.bankName(db, filled: db, found: nil, presetName: nil, custom: true), "")
        // another known BLZ replaces it, also outside "Andere FinTS-Bank"
        XCTAssertEqual(FinTsAddForm.bankName(db, filled: db, found: pb, presetName: "Postbank", custom: false), pb)
        // a preset's or search hit's own name stays
        XCTAssertEqual(FinTsAddForm.bankName("Postbank", filled: pb, found: nil, presetName: "Postbank", custom: false), "Postbank")
        XCTAssertEqual(FinTsAddForm.bankName("Meine Bank", filled: nil, found: db, presetName: "Postbank", custom: false), "Meine Bank")
        // first hit fills an empty name
        XCTAssertEqual(FinTsAddForm.bankName("", filled: nil, found: db, presetName: nil, custom: false), db)
    }

    func testEveryBankWithItsOwnConnectorHasACatalogEntry() {
        // without one the add form stays empty (Hanseatic in 0.8.7)
        let catalog = Set(bankCatalog().map(\.kind))
        for kind: ConnectorKind in [.hanseaticBank, .bhw, .kontist, .finpension, .opelDirektbank, .fordMoney] {
            XCTAssertTrue(catalog.contains(kind), "\(kind)")
        }
    }
}
