class Money:
    def __init__(self, cents: int):
        self.cents = cents

    def as_float(self) -> float:
        return self.cents / 100.0
