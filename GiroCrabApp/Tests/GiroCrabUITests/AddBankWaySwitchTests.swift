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

    func testCommerzbankOffersFinTsFirstAndKeepsPsd2() throws {
        let found = searchBanks(query: "Commerzbank", livePsd2: [], perGroupLimit: 12)
        let finTs = try XCTUnwrap(found.first { $0.mode == .finTs })
        // one row for all its bank codes: the form asks for BLZ or IBAN
        XCTAssertEqual(found.filter { $0.mode == .finTs }.count, 1)
        XCTAssertTrue(finTs.blz.isEmpty)
        XCTAssertNil(AddBankSheet.detailLine(finTs))
        XCTAssertEqual(AddBankSheet.ways(to: finTs, in: found).map(\.mode), [.finTs, .psd2])
        // the form picks the preset by the row's address and resolves a typed bank code to it
        let preset = try XCTUnwrap(fintsPresets().first { $0.blz.isEmpty && $0.url == finTs.url })
        XCTAssertEqual(preset.id, "commerzbank")
        XCTAssertFalse(preset.note.isEmpty)
        let byCode = try XCTUnwrap(lookupFintsBankByBlz(blz: "37040044"))
        XCTAssertEqual(byCode.url, finTs.url)
        XCTAssertEqual(byCode.name, finTs.name)
        // an IBAN leads straight to FinTS
        let byIban = searchBanks(query: "DE89 3704 0044 0532 0130 00", livePsd2: [], perGroupLimit: 12)
        XCTAssertEqual(byIban.map(\.id), ["fints:37040044"])
    }

    func testEveryBankWithItsOwnConnectorHasACatalogEntry() {
        // without one the add form stays empty (Hanseatic in 0.8.7)
        let catalog = Set(bankCatalog().map(\.kind))
        for kind: ConnectorKind in [.hanseaticBank, .bhw, .kontist, .finpension, .opelDirektbank, .fordMoney] {
            XCTAssertTrue(catalog.contains(kind), "\(kind)")
        }
    }
}
