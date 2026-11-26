terraform {
  required_providers {
    opensearch = {
      source  = "opensearch-project/opensearch"
      version = "2.6.0"
    }
  }
}

resource "opensearch_index" "orders" {
  name                     = "orders-v1"
  number_of_shards         = "2"
  index_knn                = true
  rollover_alias           = "orders-write"
  default_pipeline         = "orders-ingest"
  index_similarity_default = jsonencode({ type = "BM25" })
  mappings = jsonencode({
    properties = {
      title = { type = "text", analyzer = "folding" }
    }
  })
  analysis_analyzer = jsonencode({
    folding = { type = "custom", tokenizer = "standard" }
  })
  aliases = jsonencode({
    "orders-read" = {}
  })
}

resource "opensearch_index" "dynamic" {
  name     = "dynamic-v1"
  mappings = var.mapping
}

resource "opensearch_index" "named_by_local" {
  name = local.index_name
}

resource "opensearch_index_template" "legacy" {
  name = "legacy-logs"
  body = jsonencode({
    index_patterns = ["legacy-*"]
    settings       = { number_of_shards = 1 }
  })
}

resource "opensearch_composable_index_template" "orders" {
  name = "orders-template"
  body = jsonencode({
    index_patterns = ["orders-*"]
    composed_of    = ["orders-base"]
    template = {
      mappings = { properties = { sku = { type = "keyword" } } }
    }
  })
}

resource "opensearch_component_template" "base" {
  name = "orders-base"
  body = jsonencode({ template = { settings = { number_of_replicas = 1 } } })
}

resource "opensearch_ingest_pipeline" "orders" {
  name = "orders-ingest"
  body = jsonencode({ processors = [{ set = { field = "a", value = "b" } }] })
}

resource "opensearch_ingest_pipeline" "from_file" {
  name = "file-ingest"
  body = file("pipeline.json")
}

resource "opensearch_ism_policy" "hot" {
  policy_id = "hot-delete"
  body      = jsonencode({ policy = { states = [{ name = "hot" }] } })
}

resource "opensearch_data_stream" "events" {
  name = "events-stream"
}

resource "opensearch_script" "calc" {
  script_id = "calc-score"
  lang      = "painless"
  source    = "doc['x'].value"
}

resource "opensearch_script" "tmpl" {
  script_id = "orders-search"
  lang      = "mustache"
  source    = "{\"query\":{\"match_all\":{}}}"
}

resource "opensearch_sm_policy" "nightly" {
  policy_name = "nightly-snap"
  body        = jsonencode({ creation = { schedule = { cron = { expression = "0 3 * * *" } } } })
}
