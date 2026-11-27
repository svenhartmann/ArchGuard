resource "elasticsearch_index" "guess" {
  name = "no-provider-declared"
}

resource "opensearch_index" "guess" {
  name = "no-provider-declared-either"
}
