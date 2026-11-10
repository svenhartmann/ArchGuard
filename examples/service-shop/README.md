# Shop monorepo: API cycles with business context

[English](README.md) | [Deutsch](README_DE.md)

This deliberately coupled demo has **14 services in three separate Compose
stacks**: ten local APIs, two databases, Redis and RabbitMQ. ArchGuard detects 15
configured HTTP relationships, two cyclic groups of three services each and
seven separate startup dependencies.

API handlers perform product queries and orders. Data stays in memory, payments
are mock authorizations and notifications are collected locally. Database, cache
and broker adapters are not implemented; their Compose entries represent the
intended deployment environment. They appear under **Startup**, not as allegedly
observed API calls.

## Open in the ArchGuard viewer

Run all commands from the repository root:

```bash
cargo run -p archguard-cli --locked -- serve examples/service-shop --port 8789
```

Open **Services** at <http://127.0.0.1:8789/>. Analysis does not require running
containers. Three Compose files form three tree groups:

| Stack | Local APIs | Infrastructure |
| --- | --- | --- |
| Storefront | gateway, catalog, pricing, promotions | catalog-db, redis |
| Checkout | orders, payments, risk | orders-db |
| Fulfillment | inventory, shipping, notifications | rabbitmq |

An explicitly named shared network connects the stacks. Data networks remain
separate per stack. Each local service has its own build context and handler
file under `services/`.

To explore:

1. **Configured HTTP → All** shows 15 directed API relationships. **Cycles only** narrows this to the six edges in the two cyclic groups.
2. Select `orders`: **Incoming** shows `gateway` and `risk`; **Outgoing** shows `payments`, `inventory`, `shipping` and `notifications`.
3. Select an edge: the inspector shows its Compose file and environment key. For example, `risk → orders` leads to `ORDERS_API` in the checkout stack.
4. **Startup** shows seven infrastructure startup dependencies and no startup cycles. HTTP cycles are not carried into this view.

The package graph analyzes JavaScript imports separately. It cannot replace
these HTTP relationships; its KPIs describe source structure.

## Why the two cycles exist

**Catalog → pricing → promotions → catalog:** catalog needs calculated prices.
Pricing requests discount rules; promotions reads the product category back
from catalog. The return request ends at `/categories`, which does not calculate
prices. This creates an architectural cycle, not endless recursion in one request.

One way to break it: catalog passes the category with the price request, and
pricing forwards it to promotions, removing `promotions → catalog`. The tradeoff
is a larger contract and deciding who owns the validity of this product data.
A dedicated product read model is another option, with extra synchronization
and possible delays.

**Orders → payments → risk → orders:** payments requests a risk check before a
mock authorization. That check needs previous order history. Its return request
ends at the independent `/customer-history` read endpoint.

One way to break it: orders passes the required customer context with the
authorization, or risk maintains its own read model. This removes `risk → orders`
but requires decisions about data trust, freshness and additional infrastructure.
The demo intentionally retains its cycles for analysis.

`gateway`, `inventory`, `shipping` and `notifications` provide contrasting
examples: multiple incoming edges, branches and cross-stack connections without
their own API cycle. A high relationship count does not automatically imply a cycle.

## Try the APIs locally without Docker

Node 20+ and a built ArchGuard binary suffice; no npm packages are required:

```bash
cargo build -p archguard-cli --locked
node examples/service-shop/dev.js
```

The launcher reads Compose topology through ArchGuard and wires exactly those
HTTP relationships to local ports. The gateway uses `127.0.0.1:8890`; the other
nine APIs use available loopback ports. `PORT` changes the gateway port;
`ARCHGUARD_BIN` can select another built binary. `Ctrl-C` stops all APIs.

```bash
curl http://127.0.0.1:8890/products
curl 'http://127.0.0.1:8890/availability?sku=coffee'
curl -X POST http://127.0.0.1:8890/checkout \
  -H 'Content-Type: application/json' \
  -d '{"customerId":"customer-1","sku":"coffee","quantity":1}'
```

Coffee costs 1,800 cents after the demo discount. A successful order authorizes
a mock payment, reserves inventory, schedules a demo shipment and collects
notifications. Restarting resets all data. The demo has no authentication,
persistence, idempotency or compensation for distributed steps: a failure after
reservation can leave reserved inventory without a completed order.

To record real request traces, run `node examples/service-shop/scripts/record-traces.js`.
It starts the shop with `SHOP_TRACE_DIR` set, sends a product lookup, a checkout
and two failing checkouts (409 out of stock, 404 unknown product), and writes the
OTLP spans to `examples/service-shop/.archguard/otel-traces.json`;
`archguard doc examples/service-shop --arc42 --chapter 6` then draws one sequence per trace. The file is not checked in because it adds measured
edges and a `browser` caller to the analysis; delete it to return to the static view.

## Optional Docker Compose setup

The shared Dockerfile uses `additional_contexts`, requiring Compose 2.17+ and a
supporting builder ([Docker documentation](https://docs.docker.com/reference/compose-file/build/#additional_contexts)).
Start the three stacks separately to preserve their relative paths and private
networks:

```bash
docker network create archguard-service-shop-demo
docker compose -p shop-fulfillment -f examples/service-shop/stacks/fulfillment/compose.yaml up --build -d
docker compose -p shop-checkout -f examples/service-shop/stacks/checkout/compose.yaml up --build -d
docker compose -p shop-storefront -f examples/service-shop/stacks/storefront/compose.yaml up --build -d
```

Only the gateway publishes a host port, bound to loopback. The Compose and Node
launchers use the same gateway port, so run one at a time. Infrastructure
containers do not replace the demo's missing adapters.

Stop:

```bash
docker compose -p shop-storefront -f examples/service-shop/stacks/storefront/compose.yaml down
docker compose -p shop-checkout -f examples/service-shop/stacks/checkout/compose.yaml down
docker compose -p shop-fulfillment -f examples/service-shop/stacks/fulfillment/compose.yaml down
docker network rm archguard-service-shop-demo
```

## Automated checks

```bash
cargo test -p archguard-engine --locked --test service_shop_example
node --test examples/service-shop/test/*.test.js
node scripts/viewer/test_service_shop.js
```

The Rust test checks the actual Compose topology, source evidence, separate
cycles and local implementations. Node tests cover business flows and failures.
The integration test starts ten local HTTP listeners, checks price, checkout and
inventory, and verifies the same topology in the viewer. All three Compose files
were validated with `docker compose config --quiet`; container builds and runtime
were not executed.
