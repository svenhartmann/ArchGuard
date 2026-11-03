from domain import order


def log(message: str) -> None:
    print(message)


def estimate(items: list[int]) -> float:
    return order.price_cents(items).as_float()
