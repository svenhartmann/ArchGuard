from django_elasticsearch_dsl import Document, fields
from django_elasticsearch_dsl.registries import registry

from .models import Product


@registry.register_document
class ProductDocument(Document):
    name = fields.TextField(attr="name", fields={"raw": fields.KeywordField()})
    price = fields.FloatField()

    class Index:
        name = "products"
        settings = {"number_of_replicas": 0}

    class Django:
        model = Product
