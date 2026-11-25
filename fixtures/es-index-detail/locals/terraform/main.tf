module "child" {
  source = "./child"
}

resource "elasticstack_elasticsearch_index" "via_body" {
  name             = "via-body"
  mappings         = jsonencode(local.body.mappings)
  number_of_shards = local.body.settings.number_of_shards
}

resource "elasticstack_elasticsearch_index" "text_local" {
  name     = "text-local"
  mappings = local.m
}

resource "elasticstack_elasticsearch_index" "decoded" {
  name     = "decoded"
  mappings = jsondecode(local.m)
}

resource "elasticstack_elasticsearch_index" "first" {
  name             = "first"
  refresh_interval = "7s"
  analysis_analyzer = jsonencode({ a1 = { type = "standard" } })
}

resource "elasticstack_elasticsearch_index" "second" {
  name              = "second"
  analysis_analyzer = elasticstack_elasticsearch_index.first.analysis_analyzer
  refresh_interval  = elasticstack_elasticsearch_index.first.refresh_interval
}

resource "elasticstack_elasticsearch_index" "second_level" {
  name     = "second-level"
  mappings = local.two
}

resource "elasticstack_elasticsearch_index" "each" {
  for_each = { x = { x = "{}" } }
  name     = "each"
  mappings = each.value.x
}

resource "elasticstack_elasticsearch_index" "other_module" {
  name             = "other-module"
  refresh_interval = elasticstack_elasticsearch_index.inner.refresh_interval
}

resource "elasticstack_elasticsearch_index" "lookalike" {
  name     = "lookalike"
  mappings = local.q
}

resource "elasticstack_elasticsearch_index" "name_local" {
  name     = local.n
  mappings = local.m
}
