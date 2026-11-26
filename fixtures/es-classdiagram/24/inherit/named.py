from elasticsearch_dsl import Document

from .consts import ORDERS_INDEX


class Imported(Document):
    class Index:
        name = ORDERS_INDEX
