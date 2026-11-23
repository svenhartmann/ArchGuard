from elasticsearch import Elasticsearch, helpers

# import elasticsearch is only a comment here
es = Elasticsearch("http://localhost:9200")


def run(docs):
    es.index(index="events-v1", document={"a": 1})
    es.search(index="events-v1", query={"match_all": {}})
    es.count(index='events-count')
    es.delete(index="events-old", id=1)
    es.index(index="in-list", document={"index": "ignored"})
    helpers.bulk(es, [{"_index": "bulk-target", "_source": d} for d in docs])
