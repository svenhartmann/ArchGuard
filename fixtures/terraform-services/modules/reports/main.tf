# Local module: a Cloud Run service with its Cloud SQL instance.
resource "google_sql_database_instance" "reports" {
  database_version = "POSTGRES_15"
}

resource "google_cloud_run_v2_service" "reports" {
  name = "reports"
  template {
    containers {
      image = "example/reports"
      env {
        name  = "DATABASE_HOST"
        value = google_sql_database_instance.reports.private_ip_address
      }
    }
  }
}
