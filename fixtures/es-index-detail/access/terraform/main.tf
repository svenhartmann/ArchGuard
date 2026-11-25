locals {
  full = jsonencode({ properties = { a = { type = "keyword" } } })
  o    = { k = { properties = { b = { type = "text" } } } }
  arr  = [{ properties = { c = { type = "date" } } }]
}

resource "elasticstack_elasticsearch_index" "decoded_attr" {
  name     = "decoded-attr"
  mappings = jsonencode({ properties = merge(jsondecode(local.full).properties, { z = { type = "long" } }) })
}

resource "elasticstack_elasticsearch_index" "key_access" {
  name     = "key-access"
  mappings = jsonencode(local.o["k"])
}

resource "elasticstack_elasticsearch_index" "list_access" {
  name     = "list-access"
  mappings = jsonencode(local.arr[0])
}

resource "elasticstack_elasticsearch_index" "whole_attr" {
  name     = "whole-attr"
  mappings = jsondecode(local.full).properties
}

resource "elasticstack_elasticsearch_index" "missing_key" {
  name     = "missing-key"
  mappings = jsonencode(local.o["nope"])
}

resource "elasticstack_elasticsearch_index" "var_key" {
  name     = "var-key"
  mappings = jsonencode(local.o[var.k])
}
