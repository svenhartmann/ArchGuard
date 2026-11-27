terraform {
  required_providers {
    elasticstack = { source = "elastic/elasticstack" }
  }
}

locals {
  a = local.b
  b = local.a
}

resource "elasticstack_elasticsearch_index" "loop" {
  name = local.a
}
