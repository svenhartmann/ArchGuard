from elasticsearch.dsl import Document, Keyword, M, mapped_field


class LowerKeyword(Keyword):
    pass


class Rhs(Document):
    a: M[str] = mapped_field(field=Keyword())
    b: M[str] = mapped_field(exclude=True)
    c: str = LowerKeyword()
    plain: M[int]

    class Index:
        name = "rhs"


class Opaque(Document):
    d: M[str] = make_field()

    class Index:
        name = "opaque"
