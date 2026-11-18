# AWS root module: an ECS service with a managed PostgreSQL and Redis, a Lambda
# that calls the service, and two module calls (one local, one remote).
variable "payment_url" {
  type = string
}

variable "db_password" {
  type      = string
  sensitive = true
  default   = "do-not-display"
}

resource "aws_db_instance" "orders" {
  engine   = "postgres"
  db_name  = "orders"
  password = var.db_password
}

resource "aws_elasticache_replication_group" "cache" {
  description = "session cache"
}

resource "aws_secretsmanager_secret_version" "api" {
  secret_string = "do-not-display"
}

resource "aws_ecs_task_definition" "orders" {
  family = "orders"
  container_definitions = jsonencode([
    {
      name  = "orders"
      image = "example/orders:1.0"
      environment = [
        { name = "DB_HOST", value = aws_db_instance.orders.address },
        { name = "CACHE_URL", value = "redis://${aws_elasticache_replication_group.cache.primary_endpoint_address}:6379" },
        { name = "PAYMENT_URL", value = var.payment_url },
        { name = "API_TOKEN_URL", value = aws_secretsmanager_secret_version.api.arn },
        { name = "AUDIT_URL", value = "http://audit.example.org" },
      ]
      secrets = [{ name = "DB_PASSWORD", valueFrom = "arn:do-not-display" }]
    }
  ])
}

resource "aws_ecs_service" "orders" {
  name            = "orders"
  task_definition = aws_ecs_task_definition.orders.arn
}

resource "aws_lambda_function" "billing" {
  function_name = "billing"
  image_uri     = "example/billing:2"
  environment {
    variables = {
      ORDERS_HOST = aws_ecs_service.orders.name
    }
  }
}

module "network" {
  source = "terraform-aws-modules/vpc/aws"
}

module "reports" {
  source = "./modules/reports"
}

# Literal JSON in a heredoc instead of jsonencode(...).
resource "aws_ecs_task_definition" "shipping" {
  family                = "shipping"
  container_definitions = <<DEFS
[
  {
    "name": "shipping",
    "image": "example/shipping:1",
    "environment": [
      { "name": "DATABASE_URL", "value": "postgres://shipdb:5432/shipping" }
    ]
  }
]
DEFS
}

resource "aws_ecs_service" "shipping" {
  name            = "shipping"
  task_definition = aws_ecs_task_definition.shipping.arn
}
