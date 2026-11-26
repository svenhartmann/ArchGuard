from elasticsearch_dsl import Document, Keyword


class Mid(Document):
    kind = Keyword()

    class Index:
        name = "mid"


class Leaf(Mid):
    pass


class Bare(Document):
    code = Keyword()


class NoName(Document):
    class Index:
        pass
