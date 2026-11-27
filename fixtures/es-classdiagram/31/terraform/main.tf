resource "elasticstack_elasticsearch_index" "tf_vectors" {
  name = "tf-vectors"
  mappings = jsonencode({
    properties = {
      title     = { type = "text" }
      embedding = { type = "dense_vector", dims = 384, similarity = "cosine", element_type = "float", index_options = { type = "bbq_hnsw", m = 16, ef_construction = 100 } }
      body      = { type = "semantic_text", inference_id = "my-e5-endpoint" }
      summary   = { type = "semantic_text" }
      tokens    = { type = "sparse_vector" }
    }
  })
}
