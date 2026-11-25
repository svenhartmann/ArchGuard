locals {
  fields = { id = "keyword", title = "text", skip = "keyword" }
  names  = ["a", "b", "c"]
}

resource "elasticstack_elasticsearch_index" "obj_for" {
  name     = "obj-for"
  mappings = jsonencode({ properties = { for k, v in local.fields : k => { type = v } } })
}

resource "elasticstack_elasticsearch_index" "filtered" {
  name     = "filtered"
  mappings = jsonencode({ properties = { for k, v in local.fields : k => { type = v } if k != "skip" && v != "date" } })
}

resource "elasticstack_elasticsearch_index" "key_var" {
  name     = "key-var"
  mappings = jsonencode({ properties = { for f in ["x", "y"] : f => { type = "keyword" } } })
}

resource "elasticstack_elasticsearch_index" "list_for" {
  name            = "list-for"
  analysis_filter = jsonencode({ f = { type = "stop", stopwords = [for x in local.names : x if x != "b"] } })
}

resource "elasticstack_elasticsearch_index" "var_for" {
  name     = "var-for"
  mappings = jsonencode({ properties = { for k, v in var.fields : k => { type = v } } })
}

resource "elasticstack_elasticsearch_index" "each_for" {
  for_each = { x = { a = "keyword" } }
  name     = "each-for"
  mappings = jsonencode({ properties = { for k, v in each.value : k => { type = v } } })
}

resource "elasticstack_elasticsearch_index" "group_for" {
  name     = "group-for"
  mappings = jsonencode({ properties = { for k, v in local.fields : v => { type = k }... } })
}

resource "elasticstack_elasticsearch_index" "unbound" {
  name     = "unbound"
  mappings = jsonencode({ properties = { for k, v in local.fields : k => { type = w } } })
}
