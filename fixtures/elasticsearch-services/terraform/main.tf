variable "env" {
  type = string
}

resource "elasticstack_elasticsearch_ingest_pipeline" "ingest" {
  name = "orders-ingest"
  processors = [
    jsonencode({
      pipeline = {
        name = "orders-enrich"
      }
    }),
    jsonencode({
      pipeline = {
        name = var.env
      }
    }),
  ]
}

resource "elasticstack_elasticsearch_index" "orders" {
  name             = "orders-v1"
  default_pipeline = "orders-ingest"
  final_pipeline   = "orders-final"

  alias {
    name = "orders"
  }
}

resource "elasticstack_elasticsearch_index" "dynamic" {
  name = "orders-${var.env}"
}

resource "elasticstack_elasticsearch_component_template" "base" {
  name = "orders-base"
  template {
    alias {
      name = "orders-all"
    }
  }
}

resource "elasticstack_elasticsearch_index_template" "orders" {
  name           = "orders-template"
  index_patterns = ["orders-*", "orders-archive-*"]
  composed_of    = ["orders-base"]
}

resource "elasticstack_elasticsearch_index_lifecycle" "ilm" {
  name = "orders-ilm"
}

resource "elasticstack_elasticsearch_transform" "t" {
  name = "orders-rollup"
  source {
    indices = ["orders-v1"]
  }
  destination {
    index    = "orders-summary"
    pipeline = "orders-ingest"
  }
}

resource "elasticstack_elasticsearch_watch" "w" {
  watch_id = "orders-watch"
}

resource "elasticstack_elasticsearch_enrich_policy" "e" {
  name    = "orders-enrich"
  indices = ["orders-v1"]
}

resource "elasticstack_elasticsearch_snapshot_lifecycle" "s" {
  name    = "orders-nightly"
  indices = ["orders-*"]
}

resource "elasticstack_elasticsearch_script" "stored" {
  script_id = "orders-score"
  lang      = "painless"
}

resource "elasticstack_elasticsearch_script" "search" {
  script_id = "orders-search"
  lang      = "mustache"
}

module "search" {
  source = "./modules/search"
}
