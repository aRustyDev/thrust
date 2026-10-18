#!/bin/sh
# Start the Worker under `wrangler dev` (local workerd). Local only: no deploy, no login, no
# remote bindings. Configuration comes from the environment so no secret touches a file or argv:
#
#   THRUST_PORT                    listen port
#   THRUST_ADAPTERS                memory | postgres
#   THRUST_OUTBOX_FASTPATH         on | off
#   THRUST_CREATE_RATE_LIMIT       per-user create limit per window
#   THRUST_ADMIN_TOKEN             (env only) gate for /internal/*
#   DATABASE_URL                   (env only) Postgres URL for the *local* Hyperdrive binding
#
# `wrangler dev` runs the [build] command from wrangler.toml (worker-build --release).
#
# CLOUDFLARE_INCLUDE_PROCESS_ENV=true turns the WHOLE process environment into Worker bindings
# (measured: wrangler lists every env var name). So wrangler is started with a minimal, explicit
# environment (`env -i`), not with this shell's.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STATE="${THRUST_WRANGLER_STATE:-$ROOT/target/wrangler-state-${THRUST_PORT:?}}"
rm -rf "$STATE"
mkdir -p "$STATE"
cd "$ROOT/crates/thrust-worker"

if [ -n "${DATABASE_URL:-}" ]; then
  # wrangler's env form of `localConnectionString` for the Hyperdrive binding named HYPERDRIVE
  HD="${DATABASE_URL}"
else
  HD=""
fi

case "${THRUST_ADAPTERS:-memory}" in
  memory) CONFIG=wrangler.memory.toml ;;
  *) CONFIG=wrangler.toml ;;
esac

exec env -i \
  HOME="$HOME" PATH="$PATH" TMPDIR="${TMPDIR:-/tmp}" \
  CLOUDFLARE_INCLUDE_PROCESS_ENV=true WRANGLER_SEND_METRICS=false NO_COLOR=1 \
  THRUST_ADMIN_TOKEN="${THRUST_ADMIN_TOKEN:-}" \
  CLOUDFLARE_HYPERDRIVE_LOCAL_CONNECTION_STRING_HYPERDRIVE="$HD" \
  wrangler dev -c "$CONFIG" --local --ip 127.0.0.1 --port "$THRUST_PORT" \
    --persist-to "$STATE" --test-scheduled --show-interactive-dev-session=false \
    --log-level "${THRUST_WRANGLER_LOG_LEVEL:-log}" \
    --var "THRUST_TEST_ROUTES:1" \
    --var "THRUST_ADAPTERS:${THRUST_ADAPTERS:-memory}" \
    --var "THRUST_OUTBOX_FASTPATH:${THRUST_OUTBOX_FASTPATH:-on}" \
    --var "THRUST_CREATE_RATE_LIMIT:${THRUST_CREATE_RATE_LIMIT:-1000}"
