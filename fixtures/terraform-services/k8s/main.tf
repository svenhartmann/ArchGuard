# Kubernetes provider: a literal endpoint resolves through a Service selector.
resource "kubernetes_deployment" "web" {
  metadata {
    name = "web"
  }
  spec {
    template {
      metadata {
        labels = { app = "web" }
      }
      spec {
        container {
          image = "web"
          env {
            name  = "API_URL"
            value = "http://api:8080"
          }
        }
      }
    }
  }
}

resource "kubernetes_deployment" "backend" {
  metadata {
    name = "backend"
  }
  spec {
    template {
      metadata {
        labels = { app = "backend" }
      }
      spec {
        container {
          image = "backend"
        }
      }
    }
  }
}

resource "kubernetes_service" "api" {
  metadata {
    name = "api"
  }
  spec {
    selector = { app = "backend" }
  }
}
