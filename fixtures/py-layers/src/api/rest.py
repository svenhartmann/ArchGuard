from app import usecase


def summarize(items: list[int]) -> str:
    return usecase.summarize(items)
