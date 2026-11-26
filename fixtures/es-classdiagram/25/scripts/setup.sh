#!/bin/sh
HOST=http://localhost:9200
curl -X PUT "$HOST/shop-logs" -H 'Content-Type: application/json' -d '{"mappings":{"properties":{"msg":{"type":"text"}}}}'
curl -s -XPUT $HOST/_ingest/pipeline/shop-clean \
  -H 'Content-Type: application/json' \
  --data '{"processors":[{"trim":{"field":"msg"}}]}'
echo ready && curl -X PUT "$HOST/_ilm/policy/shop-ilm2" -H 'Content-Type: application/json' -d @bodies/ilm.json
curl -X PUT "$HOST/shop-from-file" --data-binary @bodies/orders.json
curl -X PUT "$HOST/_plugins/_ism/policies/shop-ism2" -d '{"policy":{"states":[]}}'
curl -X PUT "$HOST/${INDEX}" -d '{}'
curl -X PUT "$HOST/shop-outside" -d @../../../outside.json
curl -X PUT "$HOST/shop-absolute" -d @/etc/shop.json
curl "$HOST/shop-logs/_search"
curl -X DELETE "$HOST/shop-logs"
echo curl -X PUT $HOST/shop-echo -d '{}'
# curl -X PUT $HOST/shop-commented -d '{}'
mycurl -X PUT $HOST/shop-mycurl -d '{}'
