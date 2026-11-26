terraform {
  required_providers {
    elasticstack = { source = "elastic/elasticstack" }
  }
}

locals {
  names = jsondecode(file("${path.module}/big.json"))
}

resource "elasticstack_elasticsearch_index" "huge" {
  name = local.names[0]
}
