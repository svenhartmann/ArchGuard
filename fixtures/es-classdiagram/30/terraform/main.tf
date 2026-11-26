terraform {
  required_providers {
    elasticstack = { source = "elastic/elasticstack" }
  }
}

locals {
  names = jsondecode(file("${path.module}/generations.json"))
  str   = { type = "text", analyzer = "a1", fields = { keyword = { type = "keyword" } } }
  src   = merge({ for f in ["title", "body"] : f => local.str }, {})
  m     = jsonencode({ properties = merge(local.src, { id = { type = "keyword" } }) })
}

resource "elasticstack_elasticsearch_index" "current" {
  name     = local.names[0]
  mappings = local.m
  alias {
    name           = "orders"
    is_write_index = false
  }
}

resource "elasticstack_elasticsearch_index" "previous" {
  name     = local.names[1]
  mappings = local.m
  alias {
    name           = "orders"
    is_write_index = false
  }
}
