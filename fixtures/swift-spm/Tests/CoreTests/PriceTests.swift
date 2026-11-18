import XCTest
@testable import Core

final class PriceTests: XCTestCase {
    func testZero() {
        XCTAssertEqual(Price.zero(), Price(cents: 0))
    }
}
