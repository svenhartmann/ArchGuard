from domain import money
from app import service  # deliberate violation: domain must not use app


def balance(entries: list[int]) -> money.Money:
    service.log("balance")
    return money.Money(sum(entries))
