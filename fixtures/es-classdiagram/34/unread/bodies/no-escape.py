from elasticsearch import Elasticsearch

es = Elasticsearch()
es.indices.create(index="py-unknown-escape", body='{"mappings": {"properties": {"a": {"type": "te\xt"}}}}')
