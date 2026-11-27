resource "elasticstack_kibana_import_saved_objects" "dash" {
  file_contents = file("dash.ndjson")
}
