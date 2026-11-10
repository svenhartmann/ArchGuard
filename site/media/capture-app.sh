#!/usr/bin/env bash
# Capture every site screenshot and the flow video from the ArchGuard desktop app.
# Needs: an app built with `cargo build --release -p archguard-desktop --features snapshot`
# (without the feature a real window opens), ffmpeg, and the pinned checkouts from
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

# Flow video: one full turn of the World View (72 captures, 5 degrees apart, four at a time),
# then six views, 4 s each with 0.6 s fades.
mkdir -p "$tmp/turn"
for a in $(seq 0 5 355); do
  n=$(printf %03d $((a / 5))) # consecutive numbers for ffmpeg's r%03d pattern
  out="$tmp/turn" shot "r$n" "$cc" ARCHGUARD_DESKTOP_TAB=start ARCHGUARD_DESKTOP_START_MODE=architecture \
    ARCHGUARD_DESKTOP_ROTATE="$a" >/dev/null &
  [ $(((a / 5 + 1) % 4)) -eq 0 ] && wait
done
wait
ffmpeg -loglevel error -y -framerate 12 -i "$tmp/turn/r%03d.png" -vf scale=1480:920,setsar=1,fps=30 \
  -c:v libx264 -preset slow -crf 18 -pix_fmt yuv420p "$tmp/turn.mp4"
frames=(world-cbo world-package graph calls quality arc42)
inputs=(-i "$tmp/turn.mp4"); filter="[0]setsar=1,fps=30[a0];"
for i in "${!frames[@]}"; do
  inputs+=(-loop 1 -t 4 -i "$out/${frames[$i]}.png")
  filter+="[$((i + 1))]scale=1480:920,setsar=1,fps=30[a$((i + 1))];"
done
prev=a0; offset=5.4
for i in $(seq 1 ${#frames[@]}); do
  filter+="[$prev][a$i]xfade=transition=fade:duration=0.6:offset=$offset[x$i];"
  prev=x$i; offset=$(echo "$offset + 3.4" | bc)
done
ffmpeg -loglevel error -y "${inputs[@]}" -filter_complex "${filter%;}" -map "[$prev]" -an \
  -c:v libx264 -preset slow -crf 30 -pix_fmt yuv420p -movflags +faststart "$out/flow.mp4"
ffmpeg -loglevel error -y -ss 0.5 -i "$out/flow.mp4" -frames:v 1 -q:v 3 "$out/flow-poster.jpg"
echo "captured flow video"
