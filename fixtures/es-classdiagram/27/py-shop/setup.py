from elasticsearch import Elasticsearch

ORDERS = '''{"mappings": {"properties": {"title": {"type": "text"}}}}'''
RAW = r'{"settings": {"analysis": {"analyzer": {"a": {"type": "standard"}}}}}'
FMT = f'{{"mappings": {{"properties": {{"{x}": {{"type": "text"}}}}}}}}'
PIPE = '{"processors": [{"lowercase": {"field": "f"}}]}'

es = Elasticsearch()
es.indices.create(index="py-orders", body=ORDERS)
es.indices.put_index_template(name="py-tpl", body='{"index_patterns": ["py-*"], "template": {"settings": {"number_of_shards": 1}}}')
es.indices.create(index="py-raw", body=RAW)
es.ingest.put_pipeline(id="py-pipe", body=PIPE)
es.indices.create(index="py-fmt", body=FMT)
es.indices.create(index="py-concat", body=ORDERS % x)
es.indices.create(index=name_var, body=ORDERS)
es.indices.create(index="py-dict", body={"mappings": {}})
repo.tables.create(index="py-lookalike", body=ORDERS)
es.indices.exists(index="py-exists", body=ORDERS)
