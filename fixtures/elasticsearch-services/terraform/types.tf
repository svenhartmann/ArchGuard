variable "untyped_number" {
  default = 43
}

variable "untyped_bool" {
  default = true
}

variable "numbers" {
  type    = list(number)
  default = [1, 2]
}

resource "elasticstack_elasticsearch_index" "number" {
  name = var.untyped_number
}

resource "elasticstack_elasticsearch_index" "bool" {
  name = var.untyped_bool
}

resource "elasticstack_elasticsearch_index_template" "numbers" {
  name           = "numbers-template"
  index_patterns = var.numbers
}

module "number_arg" {
  source     = "./modules/named"
  index_name = 43
  patterns   = ["number-arg-*"]
}
