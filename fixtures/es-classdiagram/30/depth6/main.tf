terraform {
  required_providers {
    elasticstack = { source = "elastic/elasticstack" }
  }
}

locals {
  l1 = local.l2
  l2 = local.l3
  l3 = local.l4
  l4 = local.l5
  l5 = "five"
}

resource "elasticstack_elasticsearch_index" "five" {
  name = local.l2
}

resource "elasticstack_elasticsearch_index" "six" {
  name = local.l1
}
