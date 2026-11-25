locals {
  q = jsonencode({ properties = { c = { type = "date" } } })
}

resource "elasticstack_elasticsearch_index" "inner" {
  name             = "inner"
  mappings         = local.q
  refresh_interval = "9s"
}
