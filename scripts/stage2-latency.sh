#!/bin/sh
# STAGE 2 ONLY — do NOT run without an owner-provided Cloudflare sandbox (see research/0004,
# "Stage 2 - ready to run"). It sends real HTTP requests to a DEPLOYED Worker.
#
# Measures wall-clock latency of GET /healthz as seen from this machine:
#   - request #1 right after a fresh deployment/version (cold isolate + wasm instantiate)
#   - requests #2..N (warm)
# Repeat after >=15 minutes idle to catch an evicted isolate.
#
#   THRUST_BASE_URL=https://<worker>.<subdomain>.workers.dev scripts/stage2-latency.sh [N]
set -u
BASE="${THRUST_BASE_URL:?set THRUST_BASE_URL to the deployed Worker}"
N="${1:-30}"
i=1
while [ "$i" -le "$N" ]; do
  # connect / first byte / total, in seconds
  curl -sS -o /dev/null -w "req=$i connect=%{time_connect} ttfb=%{time_starttransfer} total=%{time_total} status=%{http_code}\n" "$BASE/healthz"
  i=$((i + 1))
done
