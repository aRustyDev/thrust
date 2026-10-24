#!/bin/sh
# Start the Worker under `wrangler dev` (local workerd). Local only: no deploy, no login, no
# remote bindings. Configuration comes from the environment so no secret touches a file or argv:
#
#   THRUST_PORT                    listen port
#   THRUST_ADAPTERS                memory | postgres
#   THRUST_OUTBOX_FASTPATH         on | off
#   THRUST_CREATE_RATE_LIMIT       per-user create limit per window
#   THRUST_ARGON2_M_KIB / _T / _P  password hashing cost (default: OWASP's minimum)
#   THRUST_SIGNIN_RATE_LIMIT       sign-in attempts per email per window
#   THRUST_CSRF_ENFORCE            on | off | default: CSRF token on cookie mutations. Unset means ON here, unlike the
#                                  production default of off, so the contract targets keep the meaning of their CSRF
#                                  tests. `default` passes no setting at all, so the wrangler config's own [vars]
#                                  value applies (off): scripts/contract.sh uses it for the `csrfoff` pass
#   THRUST_OIDC_ISSUER / _CLIENT_ID / _REDIRECT_URI, THRUST_OIDC_CLIENT_SECRET (env only, like the admin token)
#                                  the one OIDC provider; all three of the first group or none
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

if [ "$CONFIG" = wrangler.toml ]; then
  # Local Secrets Store: create the probe secret in the LOCAL state dir (no --remote, no login).
  # The value is a throwaway dummy; the probe only ever reports its length.
  env -i HOME="$HOME" PATH="$PATH" TMPDIR="${TMPDIR:-/tmp}" WRANGLER_SEND_METRICS=false NO_COLOR=1 \
    wrangler secrets-store secret create 00000000000000000000000000000000 --name probe --scopes workers \
    --value "local-dev-dummy-not-a-secret" --persist-to "$STATE" -c "$CONFIG" \
    >"$STATE/secret-create.log" 2>&1 || true
fi

# Optional vars are passed only when set, so an unset one leaves the Worker's own default in force.
set --
for v in THRUST_ARGON2_M_KIB THRUST_ARGON2_T THRUST_ARGON2_P THRUST_SIGNIN_RATE_LIMIT; do
  eval "val=\${$v:-}"
  [ -n "$val" ] && set -- "$@" --var "$v:$val"
done
# CSRF enforcement: on unless told otherwise; `default` passes nothing (the config's `[vars]` decide)
case "${THRUST_CSRF_ENFORCE:-on}" in
  default) ;;
  *) set -- "$@" --var "THRUST_CSRF_ENFORCE:${THRUST_CSRF_ENFORCE:-on}" ;;
esac

exec env -i \
  HOME="$HOME" PATH="$PATH" TMPDIR="${TMPDIR:-/tmp}" \
  CLOUDFLARE_INCLUDE_PROCESS_ENV=true WRANGLER_SEND_METRICS=false NO_COLOR=1 \
  THRUST_ADMIN_TOKEN="${THRUST_ADMIN_TOKEN:-}" \
  THRUST_OIDC_ISSUER="${THRUST_OIDC_ISSUER:-}" \
  THRUST_OIDC_CLIENT_ID="${THRUST_OIDC_CLIENT_ID:-}" \
  THRUST_OIDC_CLIENT_SECRET="${THRUST_OIDC_CLIENT_SECRET:-}" \
  THRUST_OIDC_REDIRECT_URI="${THRUST_OIDC_REDIRECT_URI:-}" \
  CLOUDFLARE_HYPERDRIVE_LOCAL_CONNECTION_STRING_HYPERDRIVE="$HD" \
  wrangler dev -c "$CONFIG" --local --ip 127.0.0.1 --port "$THRUST_PORT" \
    --persist-to "$STATE" --test-scheduled --show-interactive-dev-session=false \
    --log-level "${THRUST_WRANGLER_LOG_LEVEL:-log}" \
    --var "THRUST_TEST_ROUTES:1" \
    --var "THRUST_ADAPTERS:${THRUST_ADAPTERS:-memory}" \
    --var "THRUST_OUTBOX_FASTPATH:${THRUST_OUTBOX_FASTPATH:-on}" \
    --var "THRUST_CREATE_RATE_LIMIT:${THRUST_CREATE_RATE_LIMIT:-1000}" \
    "$@"
