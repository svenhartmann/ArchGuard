# Azure: a web app whose settings reference a PostgreSQL flexible server.
resource "azurerm_postgresql_flexible_server" "main" {
  name = "shop-db"
}

resource "azurerm_linux_web_app" "shop" {
  name = "shop"
  app_settings = {
    "DB_HOST" = azurerm_postgresql_flexible_server.main.fqdn
    "DB_NAME" = "shop"
  }
}
