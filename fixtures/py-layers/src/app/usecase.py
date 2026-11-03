from domain import order


def summarize(items: list[int]) -> str:
    return f"total={order.price_cents(items).as_float()}"
