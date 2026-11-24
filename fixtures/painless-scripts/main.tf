resource "elasticstack_elasticsearch_ingest_pipeline" "inline" {
  name = "inline"
  processors = [
    jsonencode({
      script = {
        lang   = "painless"
        source = <<-EOT
          if (ctx.a != null) {
            ctx.b = 1;
          }
          return 0;
        EOT
      }
    }),
  ]
}

resource "elasticstack_elasticsearch_ingest_pipeline" "from_file" {
  name = "from-file"
  processors = [
    jsonencode({
      script = {
        lang   = "painless"
        source = file("${path.module}/scripts/score.painless")
      }
    }),
  ]
}

resource "elasticstack_elasticsearch_ingest_pipeline" "dynamic" {
  name = "dynamic"
  processors = [
    jsonencode({
      script = {
        lang   = "painless"
        source = var.code
      }
    }),
  ]
}
