terraform {
  required_providers {
    elasticsearch = {
      source  = "phillbaker/elasticsearch"
      version = "2.0.7"
    }
  }
}

resource "elasticsearch_index" "logs" {
  name              = "logs-v1"
  number_of_shards  = "1"
  analysis_analyzer = jsonencode({ lower = { type = "custom", tokenizer = "keyword" } })
  mappings          = jsonencode({ properties = { msg = { type = "text", analyzer = "lower" } } })
}

resource "elasticsearch_index_template" "legacy" {
  name = "logs-legacy"
  body = jsonencode({ index_patterns = ["logs-*"], settings = { number_of_shards = 1 } })
}

resource "elasticsearch_composable_index_template" "logs" {
  name = "logs-template"
  body = jsonencode({
    index_patterns = ["logs-*"]
    template       = { settings = { number_of_replicas = 0 } }
  })
}

resource "elasticsearch_component_template" "base" {
  name = "logs-base"
  body = jsonencode({ template = { settings = { number_of_replicas = 0 } } })
}

resource "elasticsearch_ingest_pipeline" "logs" {
  name = "logs-ingest"
  body = jsonencode({ processors = [{ set = { field = "a", value = "b" } }] })
}

resource "elasticsearch_data_stream" "logs" {
  name = "logs-stream"
}

resource "elasticsearch_script" "calc" {
  script_id = "logs-score"
  source    = "doc['x'].value"
}

resource "elasticsearch_xpack_index_lifecycle_policy" "hot" {
  name = "logs-ilm"
  body = jsonencode({ policy = { phases = { hot = { actions = {} } } } })
}

resource "elasticsearch_xpack_snapshot_lifecycle_policy" "nightly" {
  name = "logs-snap"
  body = jsonencode({ schedule = "0 30 1 * * ?", repository = "repo" })
}

resource "elasticsearch_xpack_watch" "alarm" {
  watch_id = "logs-alarm"
  body     = jsonencode({ trigger = { schedule = { interval = "1h" } } })
}

resource "elasticsearch_opensearch_ism_policy" "hot" {
  policy_id = "logs-ism"
  body      = jsonencode({ policy = { states = [{ name = "hot" }] } })
}
