locals {
  base  = { properties = { a = { type = "keyword" } } }
  extra = { properties = { b = { type = "text" } } }
  m     = merge(local.base, local.extra)
  nolit = merge(local.base, var.extra)
  l1    = local.l2
  l2    = jsonencode({ properties = { c = { type = "date" } } })
  lx    = local.ly
  ly    = local.lx
}

resource "elasticstack_elasticsearch_index" "merged" {
  name     = "merged"
  mappings = jsonencode(local.m)
}

resource "elasticstack_elasticsearch_index" "concat" {
  name              = "concat"
  analysis_filter   = jsonencode({ f = { type = "stop", stopwords = concat(["a"], ["b"]) } })
}

resource "elasticstack_elasticsearch_index" "local_chain" {
  name     = "local-chain"
  mappings = local.l1
}

resource "elasticstack_elasticsearch_index" "a" {
  name              = "a"
  analysis_analyzer = jsonencode({ std = { type = "standard" } })
}

resource "elasticstack_elasticsearch_index" "b" {
  name              = "b"
  analysis_analyzer = elasticstack_elasticsearch_index.a.analysis_analyzer
}

resource "elasticstack_elasticsearch_index" "c" {
  name              = "c"
  analysis_analyzer = elasticstack_elasticsearch_index.b.analysis_analyzer
}

resource "elasticstack_elasticsearch_index" "d" {
  name              = "d"
  analysis_analyzer = elasticstack_elasticsearch_index.c.analysis_analyzer
}

resource "elasticstack_elasticsearch_index" "cyc_x" {
  name              = "cyc-x"
  analysis_analyzer = elasticstack_elasticsearch_index.cyc_y.analysis_analyzer
}

resource "elasticstack_elasticsearch_index" "cyc_y" {
  name              = "cyc-y"
  analysis_analyzer = elasticstack_elasticsearch_index.cyc_x.analysis_analyzer
}

resource "elasticstack_elasticsearch_index" "local_cycle" {
  name     = "local-cycle"
  mappings = local.lx
}

resource "elasticstack_elasticsearch_index" "from_var" {
  name             = "from-var"
  refresh_interval = var.refresh
}

resource "elasticstack_elasticsearch_index" "conditional" {
  name             = "conditional"
  refresh_interval = var.fast ? "1s" : "30s"
}

resource "elasticstack_elasticsearch_index" "each" {
  for_each = { x = "1s" }
  name     = "each"
  refresh_interval = each.value
}

resource "elasticstack_elasticsearch_index" "merge_var" {
  name     = "merge-var"
  mappings = jsonencode(local.nolit)
}
