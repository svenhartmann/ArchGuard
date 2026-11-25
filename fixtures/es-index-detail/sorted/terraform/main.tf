locals {
  x = { m = jsonencode({ properties = { b = { type = "text" }, a = { type = "keyword" } } }) }
}

resource "elasticstack_elasticsearch_index" "via_local" {
  name     = "via-local"
  mappings = local.x.m
}

resource "elasticstack_elasticsearch_index" "decoded" {
  name     = "decoded"
  mappings = jsondecode(local.x.m)
}

resource "elasticstack_elasticsearch_index" "direct" {
  name     = "direct"
  mappings = jsonencode({ properties = { b = { type = "text" }, a = { type = "keyword" } } })
}
