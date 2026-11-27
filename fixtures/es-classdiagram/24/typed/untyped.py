from elasticsearch.dsl import Document, Keyword, M


class Untyped(Document):
    count: M[int]

    class Index:
        name = "untyped"


class Invalid(Document):
    tag: M[Keyword]

    class Index:
        name = "invalid"
