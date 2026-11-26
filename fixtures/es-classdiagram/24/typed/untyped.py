from elasticsearch.dsl import Document, M


class Untyped(Document):
    count: M[int]

    class Index:
        name = "untyped"
