from elasticsearch.dsl import AsyncDocument, Keyword, M, NumpyDenseVector, Text, mapped_field


class Typed(AsyncDocument):
    title: M[str] = mapped_field(Text())
    tag: M[Keyword]
    vec: M[list[float]] = mapped_field(NumpyDenseVector(dims=3))
    note: str = "plain"

    class Index:
        name = "typed"
