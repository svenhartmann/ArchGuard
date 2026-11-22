#!/usr/bin/env bash
# Capture the gallery screenshots from the ArchGuard desktop app.
# Needs: an app built with `cargo build --release -p archguard-desktop --features snapshot`
# (without the feature a real window opens), and the pinned checkouts from
# examples/ (Commons Collections and Kafka with their example configuration copied in).
# Run from the repository root: site/media/capture-app.sh [path/to/archguard-desktop]
set -euo pipefail
app=${1:-target/release/archguard-desktop}
out=site/assets
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

shot() { # shot <name> <project> [VAR=value ...]
  local name=$1 project=$2; shift 2
  mkdir -p "$tmp/cfg-$name"
  env ARCHGUARD_DESKTOP_CONFIG_DIR="$tmp/cfg-$name" ARCHGUARD_DESKTOP_LANG=en \
    ARCHGUARD_DESKTOP_SNAPSHOT_MS=4000 ARCHGUARD_DESKTOP_SNAPSHOT="$out/$name.png" "$@" \
    "$app" "$project" >/dev/null 2>&1
  echo "captured $name"
}

cc=examples/checkouts/commons-collections
shot world "$cc" ARCHGUARD_DESKTOP_TAB=start ARCHGUARD_DESKTOP_START_MODE=architecture
# KPI index: 0 complexity, 1 CBO, 5 WMC, 6 LCOM4 (selecting one switches to the Explore lens).
shot world-complexity "$cc" ARCHGUARD_DESKTOP_TAB=start ARCHGUARD_DESKTOP_KPI=0
shot world-cbo "$cc" ARCHGUARD_DESKTOP_TAB=start ARCHGUARD_DESKTOP_KPI=1
shot world-wmc "$cc" ARCHGUARD_DESKTOP_TAB=start ARCHGUARD_DESKTOP_KPI=5
shot world-lcom4 "$cc" ARCHGUARD_DESKTOP_TAB=start ARCHGUARD_DESKTOP_KPI=6
shot world-package "$cc" ARCHGUARD_DESKTOP_TAB=start ARCHGUARD_DESKTOP_KPI=0 \
  ARCHGUARD_DESKTOP_WORLD=org/apache/commons/collections4/map
shot graph "$cc" ARCHGUARD_DESKTOP_TAB=graph ARCHGUARD_DESKTOP_OPEN=org/apache/commons/collections4
shot matrix "$cc" ARCHGUARD_DESKTOP_TAB=dsm
shot calls "$cc" ARCHGUARD_DESKTOP_TAB=calls
shot quality "$cc" ARCHGUARD_DESKTOP_TAB=quality
# arc42 chapters: 5 building block view, 3 context and scope.
shot arc42 "$cc" ARCHGUARD_DESKTOP_TAB=architecture ARCHGUARD_DESKTOP_CHAPTER=5
shot arc42-context examples/checkouts/eshop ARCHGUARD_DESKTOP_TAB=architecture ARCHGUARD_DESKTOP_CHAPTER=3

# Island filter: deployment 1 (the Aspire app host), connection kind 3 (inferred event flow).
shot eshop-world examples/checkouts/eshop ARCHGUARD_DESKTOP_TAB=start \
  ARCHGUARD_DESKTOP_START_MODE=services ARCHGUARD_DESKTOP_ISLAND_FILTER=1::3: ARCHGUARD_DESKTOP_ISLAND_FLAGS=cycles
# Services relation index: 0 API configuration, 3 gRPC client, 4 event flow, 5 configuration reference.
shot eshop-services examples/checkouts/eshop ARCHGUARD_DESKTOP_TAB=services \
  ARCHGUARD_DESKTOP_SVC_KIND=4 ARCHGUARD_DESKTOP_SVC_SERVICE=catalog-api
shot services examples/service-shop ARCHGUARD_DESKTOP_TAB=services \
  ARCHGUARD_DESKTOP_SVC_KIND=0 ARCHGUARD_DESKTOP_SVC_SERVICE=pricing
shot otel-grpc examples/checkouts/opentelemetry-demo ARCHGUARD_DESKTOP_TAB=services \
  ARCHGUARD_DESKTOP_SVC_KIND=3 ARCHGUARD_DESKTOP_SVC_SERVICE=checkout
shot otel-cycle examples/checkouts/opentelemetry-demo ARCHGUARD_DESKTOP_TAB=services \
  ARCHGUARD_DESKTOP_SVC_KIND=5 ARCHGUARD_DESKTOP_SVC_SERVICE=otel-collector
shot kafka-world examples/checkouts/kafka ARCHGUARD_DESKTOP_TAB=start ARCHGUARD_DESKTOP_WORLD=org/apache/kafka
