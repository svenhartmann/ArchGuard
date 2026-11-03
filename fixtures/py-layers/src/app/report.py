from domain import ledger


def statement(entries: list[int]) -> str:
    return f"balance={ledger.balance(entries).cents}"
