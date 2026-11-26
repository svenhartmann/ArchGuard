resource "elasticstack_elasticsearch_index" "docs" {
  name = "docs"

  mappings = jsonencode({
    properties = {
      title = {
        type            = "text"
        analyzer        = "idx_a"
        search_analyzer = "qry_a"
        fields          = { raw = { type = "keyword", normalizer = "fold_n" }, shingle = { type = "text", analyzer = "shingle_a" } }
      }
      body   = { type = "text" }
      same   = { type = "text", analyzer = "idx_a", search_analyzer = "twin_a" }
      author = { type = "object", properties = { name = { type = "text", analyzer = "idx_a" } } }
      lines  = { type = "nested", properties = { sku = { type = "keyword" } } }
    }
  })

  analysis_analyzer = jsonencode({
    default        = { type = "custom", tokenizer = "standard", filter = ["lowercase"] }
    default_search = { type = "custom", tokenizer = "standard", filter = ["lowercase", "trim"] }
    idx_a          = { type = "custom", tokenizer = "standard", char_filter = ["dash_map"], filter = ["lowercase", "my_stop"] }
    qry_a          = { type = "custom", tokenizer = "whitespace", filter = ["lowercase", "syn"] }
    twin_a         = { type = "custom", tokenizer = "standard", char_filter = ["dash_map"], filter = ["my_stop", "lowercase"] }
    shingle_a      = { type = "custom", tokenizer = "standard" }
    unused_a       = { type = "custom", tokenizer = "standard" }
  })
  analysis_filter = jsonencode({
    my_stop = { type = "stop" }
    syn     = { type = "synonym", synonyms = ["a, b"] }
  })
  analysis_char_filter = jsonencode({
    dash_map = { type = "mapping", mappings = ["- => _"] }
  })
  analysis_normalizer = jsonencode({
    fold_n = { type = "custom", filter = ["lowercase"] }
  })
}
