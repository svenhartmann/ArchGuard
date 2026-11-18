import Core
import Logging

func total(_ prices: [Price]) -> Price {
    var sum = Price.zero()
    for price in prices where price.cents > 0 {
        sum = Price(cents: sum.cents + price.cents)
    }
    return sum
}

print(total([Price(cents: 120)]).rounded(to: 10))
