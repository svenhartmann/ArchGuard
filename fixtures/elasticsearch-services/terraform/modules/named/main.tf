variable "index_name" {
  type    = string
  default = "named-default"
}

variable "patterns" {
  type = list(string)
}

resource "elasticstack_elasticsearch_index" "named" {
  name = var.index_name
}

resource "elasticstack_elasticsearch_index_template" "named" {
  name           = "named-template"
  index_patterns = var.patterns
}
