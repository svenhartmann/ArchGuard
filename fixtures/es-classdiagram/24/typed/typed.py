from datetime import datetime
from typing import ClassVar, Optional

from elasticsearch.dsl import AsyncDocument, InnerDoc, M, NumpyDenseVector, Text, mapped_field


class Address(InnerDoc):
    city: M[str]


class Typed(AsyncDocument):
    title: M[str] = mapped_field(Text())
    vec: M[list[float]] = mapped_field(NumpyDenseVector(dims=3))
    note: str = "plain"
    maybe: Optional[str]
    seen: M[datetime]
    home: M[Address]
    homes: M[list[Address]]
    kind: ClassVar[str] = "x"

    class Index:
        name = "typed"
