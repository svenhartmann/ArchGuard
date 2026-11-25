locals {
  x = { search = { request = { indices = ["l1"] } } }
}

resource "elasticstack_elasticsearch_index_template" "logs" {
  name           = "logs"
  index_patterns = ["logs-*"]
  template {
    mappings = jsonencode({ properties = { msg = { type = "text" } } })
    settings = jsonencode({
      index = { number_of_shards = "2", lifecycle = { name = "hot-warm" } }
      "index.refresh_interval" = "5s"
      analysis = { analyzer = { a1 = { type = "standard" } } }
    })
  }
}

resource "elasticstack_elasticsearch_component_template" "base" {
  name = "base"
  template {
    mappings = <<-EOT
      {"properties": {"ts": {"type": "date"}}}
    EOT
  }
}

resource "elasticstack_elasticsearch_index_template" "dyn" {
  name           = "dyn"
  index_patterns = ["dyn-*"]
  template {
    mappings = jsonencode(local.x)
  }
}

resource "elasticstack_elasticsearch_index" "legacy" {
  name = "legacy"
  settings {
    setting {
      name  = "index.lifecycle.name"
      value = "p9"
    }
  }
}

resource "elasticstack_elasticsearch_watch" "search" {
  watch_id = "w1"
  input = jsonencode({ search = { request = { indices = ["orders", "logs-*"] } } })
  actions = jsonencode({ log = { indices = ["not-this"] } })
}

resource "elasticstack_elasticsearch_watch" "dynamic" {
  watch_id = "w2"
  input    = jsonencode(local.x)
}

resource "elasticstack_elasticsearch_watch" "http" {
  watch_id = "w3"
  input    = jsonencode({ http = { request = { host = "h", port = 80 } } })
  actions  = jsonencode({ index = { index = { index = "also-not" } } })
}
