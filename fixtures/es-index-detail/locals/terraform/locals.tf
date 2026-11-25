locals {
  body = {
    mappings = { properties = { a = { type = "keyword" } } }
    settings = { number_of_shards = 3 }
  }
  m   = jsonencode({ properties = { b = { type = "text" } } })
  two = local.m
  n   = "named"
}
