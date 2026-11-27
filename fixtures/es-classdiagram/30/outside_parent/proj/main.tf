terraform {
  required_providers {
    elasticstack = { source = "elastic/elasticstack" }
  }
}

locals {
  names = jsondecode(file("${path.module}/../outside.json"))
  link  = jsondecode(file("${path.module}/link.json"))
}

resource "elasticstack_elasticsearch_index" "up" {
  name = local.names[0]
}

resource "elasticstack_elasticsearch_index" "link" {
  name = local.link[0]
}
