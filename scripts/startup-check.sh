#!/bin/sh
# Startup-time measurement with wrangler's own tool (local, no login, nothing is deployed):
#   `wrangler check startup` bundles the Worker (running [build]) and profiles the global-scope
#   execution in a local V8, writing a .cpuprofile. We also compute the profile's wall time.
#
#   scripts/startup-check.sh [config]        (default wrangler.toml = Cloudflare set)
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="${1:-wrangler.toml}"
OUT="$ROOT/target/startup"
mkdir -p "$OUT"
cd "$ROOT/crates/thrust-worker"
env -i HOME="$HOME" PATH="$PATH" TMPDIR="${TMPDIR:-/tmp}" WRANGLER_SEND_METRICS=false NO_COLOR=1 \
  wrangler check startup -c "$CONFIG" --outfile "$OUT/$CONFIG.cpuprofile" >"$OUT/$CONFIG.log" 2>&1
cat "$OUT/$CONFIG.log"
