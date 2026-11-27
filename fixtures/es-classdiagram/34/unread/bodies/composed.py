from elasticsearch import Elasticsearch

BODY = '{"mappings": {"properties": {"a": {"type": "' + kind + '"}}}}'
es = Elasticsearch()
es.indices.create(index="py-composed", body=BODY)
