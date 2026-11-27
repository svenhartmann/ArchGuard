from elasticsearch_dsl import Document, Keyword

from somewhere import make_base


class Items(make_base()):
    code = Keyword()

    class Index:
        name = "items"
