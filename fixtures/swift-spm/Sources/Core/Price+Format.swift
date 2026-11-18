import Foundation

extension Price: CustomStringConvertible {
    public var description: String {
        cents >= 0 ? "\(cents) ct" : "-\(-cents) ct"
    }

    public func rounded(to step: Int) -> Price {
        guard step > 0 else { return self }
        return Price(cents: (cents / step) * step)
    }
}
