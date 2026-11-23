resource "elasticstack_elasticsearch_ingest_pipeline" "failing" {
  name = "orders-failing"
  processors = [
    jsonencode({
      set = {
        field = "a"
        value = "b"
      }
    }),
  ]
  on_failure = [
    jsonencode({
      pipeline = {
        name = "orders-fallback"
      }
    }),
  ]
}

resource "elasticstack_elasticsearch_ingest_pipeline" "from_variable" {
  name       = "orders-from-variable"
  processors = var.processors
}

resource "elasticstack_elasticsearch_ingest_pipeline" "wrapped" {
  name       = "orders-wrapped"
  processors = [jsonencode(var.processor)]
}
