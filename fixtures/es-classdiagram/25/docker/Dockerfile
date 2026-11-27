FROM alpine
RUN apk add curl && curl -X PUT http://es:9200/shop-docker -H 'Content-Type: application/json' -d '{"settings":{"number_of_shards":1}}'
RUN curl -X PUT http://es:9200/_component_template/shop-docker-comp \
    -H 'Content-Type: application/json' \
    -d '{"template":{"settings":{"number_of_replicas":0}}}'
