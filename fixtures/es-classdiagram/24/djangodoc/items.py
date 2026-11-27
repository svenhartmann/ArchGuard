from django_elasticsearch_dsl import Document, fields

from .models import Item


class ItemDocument(Document):
    name = fields.TextField()

    class Index:
        name = "items"

    class Django:
        model = Item
        fields = ["name", "price"]
