terraform {
  required_providers {
    opensearch    = { source = "acme/opensearch" }
    elasticsearch = { source = "elastic/elasticsearch" }
  }
}

resource "opensearch_index" "other" {
  name = "other-source-os"
}

resource "elasticsearch_index" "other" {
  name = "other-source-es"
}
