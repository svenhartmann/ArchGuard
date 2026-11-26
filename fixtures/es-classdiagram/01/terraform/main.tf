resource "elasticstack_elasticsearch_index" "products" {
  name = "products"

  mappings = jsonencode({
    properties = { title = { type = "text", analyzer = "title_fold" } }
  })

  analysis_analyzer = jsonencode({
    title_fold = {
      type        = "custom"
      char_filter = ["html_strip", "dash_map"]
      tokenizer   = "standard"
      filter      = ["lowercase", "my_stop", "icu_folding", "syn"]
    }
    prefix_search = {
      tokenizer = "prefix_tok"
      filter    = [{ type = "length", min = 2 }]
    }
    broken = "not an object"
  })
  analysis_tokenizer = jsonencode({
    prefix_tok = { type = "edge_ngram", min_gram = 2, max_gram = 10, token_chars = ["letter", "digit"] }
  })
  analysis_filter = jsonencode({
    my_stop = { type = "stop", stopwords = "_german_" }
    syn     = { type = "synonym_graph", synonyms_path = "analysis/synonyms.txt", updateable = true }
  })
  analysis_char_filter = jsonencode({
    dash_map = { type = "mapping", mappings = ["- => _"] }
  })
  analysis_normalizer = jsonencode({
    lower_fold = { type = "custom", filter = ["lowercase", "asciifolding"] }
  })
}

resource "elasticstack_elasticsearch_index_template" "logs" {
  name           = "logs"
  index_patterns = ["logs-*"]
  template {
    settings = jsonencode({
      index = {
        analysis = {
          analyzer    = { log_a = { type = "custom", tokenizer = "log_tok", filter = ["lowercase"] } }
          tokenizer   = { log_tok = { type = "pattern", pattern = "," } }
          filter      = { log_f = { type = "trim" } }
          char_filter = { log_c = { type = "html_strip" } }
          normalizer  = { log_n = { type = "custom", filter = ["lowercase"] } }
        }
      }
    })
  }
}

resource "elasticstack_elasticsearch_component_template" "base" {
  name = "base"
  template {
    settings = jsonencode({
      index = {
        analysis = {
          analyzer    = { base_a = { type = "custom", tokenizer = "base_tok" } }
          tokenizer   = { base_tok = { type = "whitespace" } }
          filter      = { base_f = { type = "uppercase" } }
          char_filter = { base_c = { type = "mapping", mappings = ["a => b"] } }
          normalizer  = { base_n = { type = "custom", filter = ["base_f"] } }
        }
      }
    })
  }
}
