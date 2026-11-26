from elasticsearch_dsl import Document, Keyword


class BaseDoc(Document):
    code = Keyword()

    class Index:
        name = "base-docs"
        settings = {"number_of_shards": 1}
