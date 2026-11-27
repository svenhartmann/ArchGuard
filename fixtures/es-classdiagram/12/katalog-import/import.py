from elasticsearch import Elasticsearch

es = Elasticsearch("http://localhost:9200")


def run(doc):
    es.index(index="produkte_aktuell", document=doc)
