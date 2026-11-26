terraform {
  required_providers {
    elasticsearch = { source = "phillbaker/elasticsearch" }
    opensearch    = { source = "opensearch-project/opensearch" }
  }
}

# Security and monitor objects are out of scope; the AWS domain is no search artifact.
resource "elasticsearch_xpack_user" "svc" {
  username = "orders-user"
}

resource "opensearch_role" "reader" {
  role_name = "orders-reader"
}

resource "aws_opensearch_domain" "main" {
  domain_name = "orders-domain"
}
