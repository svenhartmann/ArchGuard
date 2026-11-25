locals {
  fields = { id = "keyword", title = "text" }
  suffix = "raw"
  names  = ["a", "b"]
}

resource "elasticstack_elasticsearch_index" "key_interp" {
  name     = "key-interp"
  mappings = jsonencode({ properties = { for k, v in local.fields : "${k}_${local.suffix}" => { type = v } } })
}

resource "elasticstack_elasticsearch_index" "value_interp" {
  name     = "value-interp"
  mappings = jsonencode({ properties = { for k in local.names : k => { type = "text", analyzer = "an_${k}" } } })
}

resource "elasticstack_elasticsearch_index" "number_interp" {
  name     = "number-interp"
  mappings = jsonencode({ properties = { x = { type = "keyword", ignore_above = "${256}" } } })
}

resource "elasticstack_elasticsearch_index" "var_interp" {
  name     = "var-interp"
  mappings = jsonencode({ properties = { "f_${var.suffix}" = { type = "keyword" } } })
}

resource "elasticstack_elasticsearch_index" "each_interp" {
  for_each = { x = "y" }
  name     = "each-interp"
  mappings = jsonencode({ properties = { "f_${each.value}" = { type = "keyword" } } })
}

resource "elasticstack_elasticsearch_index" "directive" {
  name     = "directive"
  mappings = jsonencode({ properties = { "f_%{ if true }a%{ endif }" = { type = "keyword" } } })
}
