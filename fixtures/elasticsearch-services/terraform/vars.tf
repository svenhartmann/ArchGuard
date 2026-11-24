variable "root_index" {
  type    = string
  default = "orders-v3"
}

variable "secret_index" {
  type      = string
  default   = "secret-must-not-appear"
  sensitive = true
}

variable "unset_index" {
  type = string
}

variable "root_patterns" {
  type    = list(string)
  default = ["vars-a-*", "vars-b-*"]
}

variable "module_index" {
  type    = string
  default = "orders-v4"
}

locals {
  local_index = "local-must-not-appear"
}

resource "elasticstack_elasticsearch_index" "from_default" {
  name = var.root_index
}

resource "elasticstack_elasticsearch_index" "secret" {
  name = var.secret_index
}

resource "elasticstack_elasticsearch_index" "unset" {
  name = var.unset_index
}

resource "elasticstack_elasticsearch_index" "from_local" {
  name = local.local_index
}

resource "elasticstack_elasticsearch_index_template" "from_list" {
  name           = "vars-template"
  index_patterns = var.root_patterns
}

module "literal_arg" {
  source     = "./modules/named"
  index_name = "orders-v2"
  patterns   = var.root_patterns
}

module "chained_arg" {
  source     = "./modules/named"
  index_name = var.module_index
  patterns   = ["chained-*"]
}

module "defaulted" {
  source = "./modules/named"
}

module "dynamic_arg" {
  source     = "./modules/named"
  index_name = local.local_index
  patterns   = ["dynamic-*"]
}
