terraform {
  required_providers {
    opensearch = { source = "opensearch-project/opensearch" }
  }
}

locals {
  names = jsondecode(file("${path.module}/names.json"))
  pipe  = "local-pipe"
}

resource "opensearch_index" "a" {
  name = local.names[0]
}

resource "opensearch_ingest_pipeline" "p" {
  name = local.pipe
  body = "{}"
}
