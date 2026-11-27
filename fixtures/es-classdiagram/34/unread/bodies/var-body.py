from elasticsearch import Elasticsearch

es = Elasticsearch()
es.indices.create(index="py-var", body=load_body())
