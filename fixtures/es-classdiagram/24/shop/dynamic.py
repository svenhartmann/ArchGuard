import os

from elasticsearch_dsl import Document, Keyword, Text


def pick():
    return "standard"


class Partial(Document):
    title = Text(analyzer=pick())
    code = Keyword()

    class Index:
        name = "partial"
        settings = {"number_of_shards": int(os.environ["SHARDS"])}


class Nameless(Document):
    code = Keyword()

    class Index:
        name = os.environ["ENV"] + "-nameless"
