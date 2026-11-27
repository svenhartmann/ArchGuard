curl -X PUT "$ES/_index_template/${TEMPLATE}" -d '{"index_patterns":["x-*"],"template":{}}'
