import XCTest
import GiroCrabCore
@testable import GiroCrabUI

final class HeaderPieSlicesTests: XCTestCase {
    func testCategorizedExpenseTotalsExcludeUncategorizedAndNonpositiveShares() {
        let totals = [
            CategoryFlowTotal(categoryId: nil, name: "Uncategorized", colorHex: "", cents: 100, bookingCount: 1),
            CategoryFlowTotal(categoryId: 1, name: "Food", colorHex: "", cents: 200, bookingCount: 1),
            CategoryFlowTotal(categoryId: 2, name: "Credit", colorHex: "", cents: -50, bookingCount: 1),
            CategoryFlowTotal(categoryId: 3, name: "Zero", colorHex: "", cents: 0, bookingCount: 1),
        ]
        XCTAssertEqual(HeaderPieSlices.categorizedExpenses(totals), [.init(id: "1", label: "Food", cents: 200)])
        XCTAssertTrue(HeaderPieSlices.categorizedExpenses([]).isEmpty)
    }
    private func cat(_ id: Int64, parent: Int64?, name: String, own: Int64, total: Int64, depth: UInt32) -> GiroCrabCore.Category {
        GiroCrabCore.Category(id: id, parentId: parent, name: name, sfSymbol: "tag", colorHex: "", sortOrder: 0, depth: depth,
                              path: name, ownTotalCents: own, totalCents: total, transactionCount: 1, budgetCents: nil)
    }

    func testSubcategoriesShareTheParentsSide() {
        let cats = [
            cat(1, parent: nil, name: "Wohnen", own: -100, total: -1100, depth: 0),
            cat(2, parent: 1, name: "Miete", own: -900, total: -900, depth: 1),
            cat(3, parent: 1, name: "Strom", own: -150, total: -150, depth: 1),
            cat(4, parent: 1, name: "Erstattung", own: 50, total: 50, depth: 1),
            cat(5, parent: 2, name: "Garage", own: -1, total: -1, depth: 2),
        ]
        let parts = HeaderPieSlices.subcategories(of: 1, in: cats)
        XCTAssertEqual(parts.map(\.id).sorted(), ["2", "3", "own"])
        XCTAssertEqual(parts.first { $0.id == "2" }?.cents, 900)
        XCTAssertTrue(HeaderPieSlices.subcategories(of: 99, in: cats).isEmpty)
    }

    func testExpensesUseTopLevelTotals() {
        let cats = [
            cat(1, parent: nil, name: "Wohnen", own: 0, total: -500, depth: 0),
            cat(2, parent: 1, name: "Miete", own: -500, total: -500, depth: 1),
            cat(3, parent: nil, name: "Gehalt", own: 2000, total: 2000, depth: 0),
        ]
        XCTAssertEqual(HeaderPieSlices.expenses(cats), [HeaderPieSlices.Part(id: "1", label: "Wohnen", cents: 500)])
    }

    func testFoldKeepsLargestAndSumsTheRest() throws {
        let parts = (1...6).map { HeaderPieSlices.Part(id: "\($0)", label: "K\($0)", cents: Int64($0 * 100)) }
        let slices = try XCTUnwrap(HeaderPieSlices.fold(parts, limit: 4))
        XCTAssertEqual(slices.map(\.id), ["6", "5", "4", "rest"])
        XCTAssertEqual(slices.last?.cents, 600)
        XCTAssertEqual(HeaderPieSlices.fold(parts, limit: 6)?.count, 6)
        XCTAssertEqual(slices.reduce(0) { $0 + $1.cents }, 2100)
    }

    func testFoldRejectsOverflowAndAcceptsEmptyInput() {
        XCTAssertEqual(HeaderPieSlices.fold([], limit: 4)?.count, 0)
        let parts = [Int64.max, 1].enumerated().map { HeaderPieSlices.Part(id: "\($0.offset)", label: "Part", cents: $0.element) }
        XCTAssertNil(HeaderPieSlices.fold(parts, limit: 1))
        XCTAssertNil(HeaderPieSlices.fold(parts, limit: 9))
        XCTAssertNil(HeaderPieSlices.total([Int64.max, 1]))
    }
}
