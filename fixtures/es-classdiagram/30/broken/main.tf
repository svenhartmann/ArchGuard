terraform {
  required_providers {
    elasticstack = { source = "elastic/elasticstack" }
  }
}

locals {
  names = jsondecode(file("${path.module}/broken.json"))
}

resource "elasticstack_elasticsearch_index" "bad" {
  name = local.names[0]
}
