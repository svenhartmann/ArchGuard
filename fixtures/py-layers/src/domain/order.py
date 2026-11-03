from domain import money


def price_cents(items: list[int]) -> money.Money:
    return money.Money(sum(items))
