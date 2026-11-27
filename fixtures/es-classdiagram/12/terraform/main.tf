resource "elasticstack_elasticsearch_ingest_pipeline" "norm" {
  name = "normalisieren"
  processors = [
    jsonencode({ lowercase = { field = "titel" } }),
  ]
}

resource "elasticstack_elasticsearch_index_template" "produkte" {
  name           = "produkte-*"
  index_patterns = ["produkte-*"]
  template {
    settings = jsonencode({ index = { number_of_shards = "3" } })
  }
}

resource "elasticstack_elasticsearch_index" "aktuell" {
  name             = "produkte_aktuell"
  default_pipeline = "normalisieren"

  alias {
    name = "produkte"
  }

  settings {
    setting {
      name  = "index.number_of_shards"
      value = "3"
    }
    setting {
      name  = "index.number_of_replicas"
      value = "1"
    }
    setting {
      name  = "index.lifecycle.name"
      value = "produkte-ilm"
    }
  }

  mappings = jsonencode({
    properties = {
      id    = { type = "keyword" }
      titel = {
        type            = "text"
        analyzer        = "german_custom"
        search_analyzer = "german_search"
        fields          = { raw = { type = "keyword", normalizer = "lower" } }
      }
      beschreibung = { type = "text", analyzer = "german_custom" }
      preis        = { type = "scaled_float" }
      hersteller   = { type = "object", properties = { name = { type = "text" }, land = { type = "keyword" } } }
    }
  })

  analysis_analyzer = jsonencode({
    german_custom = { type = "custom", tokenizer = "standard", filter = ["lowercase", "german_stop"] }
    german_search = { type = "custom", tokenizer = "standard", filter = ["lowercase", "german_stop", "synonyme"] }
    autocomplete  = { type = "custom", tokenizer = "autocomplete_tok", filter = ["lowercase"] }
  })
  analysis_tokenizer = jsonencode({
    autocomplete_tok = { type = "edge_ngram", min_gram = 2, max_gram = 15 }
  })
  analysis_filter = jsonencode({
    german_stop = { type = "stop", stopwords = "_german_" }
    synonyme    = { type = "synonym_graph", synonyms_path = "analysis/synonyme.txt" }
  })
  analysis_normalizer = jsonencode({
    lower = { type = "custom", filter = ["lowercase"] }
  })
}

resource "elasticstack_elasticsearch_index" "archiv" {
  name = "produkte-2026-01"
}
