from other_library import Text


class Document:
    """Our own base class, not elasticsearch-dsl."""


class Fake(Document):
    title = Text()

    class Index:
        name = "fake-index"
