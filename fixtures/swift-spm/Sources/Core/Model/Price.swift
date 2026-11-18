public struct Price: Equatable {
    public let cents: Int

    public init(cents: Int) {
        self.cents = cents
    }

    public static func zero() -> Price {
        Price(cents: 0)
    }
}

public protocol Pricing {
    func price() -> Price
}
