from elasticsearch_dsl import Text
from vendor_lib import Thing

from .base import BaseDoc


class Child(BaseDoc):
    title = Text()


class Lost(Thing):
    body = Text()
