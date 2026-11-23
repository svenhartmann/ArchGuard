resource "elasticstack_kibana_data_view" "orders" {
  data_view = {
    name  = "orders-view"
    title = "orders-*, orders-archive-*,-orders-tmp*"
  }
}

resource "elasticstack_kibana_data_view" "dynamic" {
  data_view = {
    name  = "logs-view"
    title = var.logs_title
  }
}

resource "elasticstack_kibana_alerting_rule" "query" {
  name         = "orders-alert"
  rule_type_id = ".es-query"
  params = jsonencode({
    index     = ["orders-*", "orders-archive-*"]
    threshold = [1]
  })
}

resource "elasticstack_kibana_alerting_rule" "json" {
  name         = "orders-json-alert"
  rule_type_id = "logs.alert.document.count"
  params       = "{\"index\":\"orders-logs-*\"}"
}

resource "elasticstack_kibana_alerting_rule" "dynamic" {
  name         = "orders-dynamic-alert"
  rule_type_id = ".es-query"
  params       = jsonencode({ index = [var.index] })
}

resource "elasticstack_kibana_import_saved_objects" "dashboards" {
  file_contents = file("dashboards.ndjson")
}

resource "elasticstack_kibana_alerting_rule" "search_source" {
  name         = "orders-source-alert"
  rule_type_id = ".es-query"
  params = jsonencode({
    searchType = "searchSource"
    searchConfiguration = {
      index = "00000000-dataview-id"
    }
  })
}
