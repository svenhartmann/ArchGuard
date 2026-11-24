resource "elasticstack_elasticsearch_ingest_pipeline" "ingest" {
  name = "known-pipeline"
}

resource "elasticstack_elasticsearch_index" "orders" {
  name             = "orders-v1"
  default_pipeline = "known-pipeline"
  final_pipeline   = "missing-pipeline"

  alias {
    name = "orders"
  }
}

resource "elasticstack_elasticsearch_index" "audit" {
  name = "audit-log"
}

resource "elasticstack_elasticsearch_index_template" "logs" {
  name           = "logs"
  index_patterns = ["logs-*", "-logs-tmp*", "m*trics-x"]
}

resource "elasticstack_kibana_data_view" "good" {
  data_view = {
    name  = "good"
    title = "logs-app-*,orders,-nothing-*"
  }
}

resource "elasticstack_kibana_data_view" "bad" {
  data_view = {
    name  = "bad"
    title = "ghost-*"
  }
}

resource "elasticstack_kibana_alerting_rule" "good" {
  name         = "good"
  rule_type_id = ".es-query"
  params       = jsonencode({ index = ["audit-log", "m-metrics-x"] })
}

resource "elasticstack_kibana_alerting_rule" "bad" {
  name         = "bad"
  rule_type_id = ".es-query"
  params       = jsonencode({ index = ["phantom"] })
}
