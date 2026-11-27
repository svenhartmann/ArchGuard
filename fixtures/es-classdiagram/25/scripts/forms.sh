#!/bin/sh
HOST=http://localhost:9200
curl -X PUT "$HOST/shop-attached" -d@bodies/attached.json
curl -X PUT "$HOST/shop-glued" -d'{"settings":{"number_of_shards":4}}'
curl -X PUT "$HOST/shop-eq" --data='{"settings":{"number_of_shards":5}}'
curl -X PUT "$HOST/shop-json" --json '{"mappings":{"properties":{"j":{"type":"keyword"}}}}'
curl -X PUT "$HOST/shop-lines" -H 'Content-Type: application/json' -d '{
  "mappings": {"properties": {"m": {"type": "text"}}}
}'
echo "usage:
curl -X PUT $HOST/shop-usage -d '{}'
"
