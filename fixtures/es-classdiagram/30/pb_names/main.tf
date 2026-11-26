terraform {
  required_providers {
    elasticsearch = { source = "phillbaker/elasticsearch" }
  }
}

locals {
  idx = "pb-from-local"
}

resource "elasticsearch_index" "a" {
  name = local.idx
}
