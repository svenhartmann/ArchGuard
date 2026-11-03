from app import service


def handle(items: list[int]) -> str:
    return str(service.estimate(items))
