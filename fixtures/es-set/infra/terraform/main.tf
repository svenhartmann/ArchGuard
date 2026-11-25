resource "elasticstack_elasticsearch_ingest_pipeline" "p1" {
  name = "p1"
}

resource "elasticstack_elasticsearch_index" "orders" {
  name             = "orders"
  default_pipeline = "p1"
}

resource "elasticstack_elasticsearch_index" "audit" {
  name = "audit"
}
