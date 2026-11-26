from elasticsearch.dsl import Document, Keyword, M, mapped_field


class LowerKeyword(Keyword):
    pass


class Rhs(Document):
    a: M[str] = mapped_field(field=Keyword())
    b: M[str] = mapped_field(exclude=True)
    c: str = LowerKeyword()
    plain: M[int]
    e: M[str] = mapped_field(Keyword(), exclude=True)
    f: M[str] = mapped_field(Keyword(), es_name="renamed")

    class Index:
        name = "rhs"


class Opaque(Document):
    d: M[str] = make_field()

    class Index:
        name = "opaque"


class Subclassed(Document):
    c: str = LowerKeyword(normalizer="lower")

    class Index:
        name = "subclassed"


class Positional(Document):
    x: M[str] = mapped_field("named")

    class Index:
        name = "positional"
