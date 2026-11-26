terraform {
  required_providers {
    opensearch = { source = "registry.terraform.io/opensearch-project/opensearch" }
  }
}

resource "opensearch_index" "hosted" {
  name = "host-form"
}
