variable "s" {
  type = number
}

locals {
  m = jsonencode({ properties = { x = { type = "keyword" } } })
}

resource "elasticstack_elasticsearch_index" "orders" {
  name = "orders"
  mappings = jsonencode({
    properties = {
      id    = { type = "keyword" }
      title = {
        type     = "text"
        analyzer = "german_custom"
        fields   = { raw = { type = "keyword", normalizer = "lower" } }
      }
      customer = { properties = { name = { type = "text" } } }
      lines    = { type = "nested", properties = { sku = { type = "keyword" } } }
    }
  })
  number_of_shards   = 1
  number_of_replicas = 2
  refresh_interval   = "30s"
  default_pipeline   = "p1"
  analysis_analyzer  = jsonencode({ german_custom = { type = "custom", tokenizer = "standard" } })
  analysis_normalizer = jsonencode({ lower = { type = "custom", filter = ["lowercase"] } })
}

resource "elasticstack_elasticsearch_index" "heredoc" {
  name     = "heredoc"
  mappings = <<-EOT
    {"properties":{"ts":{"type":"date"}}}
  EOT
}

resource "elasticstack_elasticsearch_index" "computed" {
  name             = "computed"
  mappings         = local.m
  number_of_shards = var.s
  refresh_interval = "1s"
}

resource "elasticstack_elasticsearch_index" "plain" {
  name = "plain"
}

resource "elasticstack_elasticsearch_transform" "t" {
  name             = "t"
  mappings         = jsonencode({ properties = { z = { type = "keyword" } } })
  number_of_shards = 3
}

resource "elasticstack_elasticsearch_index" "aliased" {
  name = "aliased"
  alias {
    name             = "al"
    mappings         = jsonencode({ properties = { z = { type = "keyword" } } })
    number_of_shards = 3
  }
}
