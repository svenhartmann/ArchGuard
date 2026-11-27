from elasticsearch import dsl
from opensearchpy import Document as OsDocument
from opensearchpy import Keyword


class Event(OsDocument):
    kind = Keyword()

    class Index:
        name = "events"


class Note(dsl.Document):
    body = dsl.Text()

    class Index:
        name = "notes"
