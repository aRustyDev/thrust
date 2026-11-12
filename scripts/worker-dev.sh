#!/bin/sh
# Start the Worker under `wrangler dev` (local workerd). Local only: no deploy, no login, no
# remote bindings. Configuration comes from the environment so no secret touches a file or argv:
#
#   THRUST_PORT                    listen port
#   THRUST_ADAPTERS                memory | postgres
#   THRUST_OUTBOX_FASTPATH         on | off
#   THRUST_CREATE_RATE_LIMIT       per-user create limit per window. On the Cloudflare set (THRUST_ADAPTERS=cloudflare) it is the
#                                  `RL_CREATE` binding's limit, written into a throwaway copy of the config (see below)
#   THRUST_WORKER_CREATE_LIMIT     the `RL_CREATE` binding's limit when it is not to be THRUST_CREATE_RATE_LIMIT. scripts/contract.sh
#                                  sets only the latter, so the contract suite expects what the binding has; setting this one
#                                  too is how a control shows that the suite fails when the binding is above what it expects
#   THRUST_ARGON2_M_KIB / _T / _P  password hashing cost (default: OWASP's minimum)
#   THRUST_SIGNIN_RATE_LIMIT       sign-in attempts per email per window (the `RL_SIGNIN` binding's limit on the Cloudflare set)
#   THRUST_V1_RATE_LIMIT           a key's allowance on /api/v1, in Plane's spelling (`1000000/minute`); unset leaves the
#                                  Worker's default, Plane's 60/minute. On the Cloudflare set its count is the `RL_V1` binding's
#                                  limit (the period is the binding's 60 seconds, so `/minute`), written like the others
#   THRUST_CSRF_ENFORCE            on | off | default: CSRF token on cookie mutations. Unset means ON here, unlike the
#                                  production default of off, so the contract targets keep the meaning of their CSRF
#                                  tests. `default` passes no setting at all, so the wrangler config's own [vars]
#                                  value applies (off): scripts/contract.sh uses it for the `csrfoff` pass
#   THRUST_OIDC_ISSUER / _CLIENT_ID / _REDIRECT_URI, THRUST_OIDC_CLIENT_SECRET (env only, like the admin token)
#                                  the one OIDC provider; all three of the first group or none
#   THRUST_ASSETS_DIR              a directory of static files the Worker serves beside the API: the built app,
#                                  `web/apps/web/build/client`. Written as an `[assets]` block into the throwaway copy of the config
#                                  (see below), never into the shared wrangler.toml, which the contract cells use as they are. Unset
#                                  serves no files. An absolute path, to a directory that holds an index.html (exit 2 otherwise)
#   THRUST_ADMIN_TOKEN             (env only) gate for /internal/*
#   DATABASE_URL                   (env only) Postgres URL for the *local* Hyperdrive binding
#
# `wrangler dev` runs the [build] command from wrangler.toml (worker-build --release).
#
# The Cloudflare set's rate limits are `[[ratelimits]]` bindings (ADR-0033), and a binding's limit is not a var: `--var` cannot set
# it. So when the run wants a create or sign-in limit, this script writes them into a throwaway copy of wrangler.toml
# (`.wrangler.run-<pid>.toml`, beside the original so that its relative paths mean the same, ignored by git) and starts wrangler
# from that, and removes it when it ends. It fails (exit 3) if the config has no block for a binding it was asked to set, so a
# renamed binding cannot leave a run with a limit nobody chose. wrangler.toml itself keeps the app's defaults.
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

# The rate limits of the Cloudflare set: write the ones this run wants into a throwaway copy of the config (see the header).
RUN_CONFIG=""
cleanup() {
  [ -z "$RUN_CONFIG" ] || rm -f "$RUN_CONFIG"
}
trap cleanup EXIT INT TERM

# set_limit BINDING LIMIT < config > config: the `limit` of the `[[ratelimits]]` block whose `name` is BINDING. Exits 3 when there is
# no such block, which is a binding this script cannot set and a run that would otherwise carry a limit nobody chose.
set_limit() {
  awk -v name="$1" -v limit="$2" '
    /^\[\[/ { cur = "" }
    $0 == "name = \"" name "\"" { cur = name }
    cur == name && /^simple = / { if (sub(/limit = [0-9]+/, "limit = " limit)) done = 1; cur = "" }
    { print }
    END { exit done ? 0 : 3 }'
}

if [ "$CONFIG" = wrangler.toml ]; then
  CREATE_LIMIT="${THRUST_WORKER_CREATE_LIMIT:-${THRUST_CREATE_RATE_LIMIT:-}}"
  SIGNIN_LIMIT="${THRUST_SIGNIN_RATE_LIMIT:-}"
  # `1000000/minute`: the count is the binding's limit; the binding's period is 60 seconds, so another period is refused here
  # rather than left to give a run a limit that is not the one asked for
  V1_LIMIT=""
  case "${THRUST_V1_RATE_LIMIT:-}" in
    "") ;;
    *[0-9]/minute) V1_LIMIT="${THRUST_V1_RATE_LIMIT%%/*}" ;;
    *) echo "worker-dev: on the Cloudflare set THRUST_V1_RATE_LIMIT is <count>/minute (the RL_V1 binding's period is 60 seconds), not '$THRUST_V1_RATE_LIMIT'" >&2; exit 2 ;;
  esac
  for limit in "$CREATE_LIMIT" "$SIGNIN_LIMIT" "$V1_LIMIT"; do
    case "$limit" in
      *[!0-9]*) echo "worker-dev: a rate limit must be a number, not '$limit'" >&2; exit 2 ;;
    esac
  done
  if [ -n "$CREATE_LIMIT" ] || [ -n "$SIGNIN_LIMIT" ] || [ -n "$V1_LIMIT" ]; then
    cp "$CONFIG" "$STATE/config.toml"
    if [ -n "$CREATE_LIMIT" ]; then
      set_limit RL_CREATE "$CREATE_LIMIT" <"$STATE/config.toml" >"$STATE/config.next" ||
        { echo "worker-dev: $CONFIG has no [[ratelimits]] block named RL_CREATE" >&2; exit 3; }
      mv "$STATE/config.next" "$STATE/config.toml"
    fi
    if [ -n "$SIGNIN_LIMIT" ]; then
      set_limit RL_SIGNIN "$SIGNIN_LIMIT" <"$STATE/config.toml" >"$STATE/config.next" ||
        { echo "worker-dev: $CONFIG has no [[ratelimits]] block named RL_SIGNIN" >&2; exit 3; }
      mv "$STATE/config.next" "$STATE/config.toml"
    fi
    if [ -n "$V1_LIMIT" ]; then
      set_limit RL_V1 "$V1_LIMIT" <"$STATE/config.toml" >"$STATE/config.next" ||
        { echo "worker-dev: $CONFIG has no [[ratelimits]] block named RL_V1" >&2; exit 3; }
      mv "$STATE/config.next" "$STATE/config.toml"
    fi
    RUN_CONFIG=".wrangler.run-$$.toml"
    cp "$STATE/config.toml" "$RUN_CONFIG"
    CONFIG="$RUN_CONFIG"
    echo "worker-dev: rate limit bindings from $CONFIG: RL_CREATE=${CREATE_LIMIT:-default} RL_SIGNIN=${SIGNIN_LIMIT:-default} RL_V1=${V1_LIMIT:-default}"
  fi
fi

# The app's files (ADR-0011): one single-page app at `/`, every path the Worker handles run first, in both forms (a browser
# navigation to `/auth` or `/api` is otherwise answered with index.html and never reaches the Worker). A missing hashed asset is
# answered 200 text/html by the single-page fallback, so a check that files load must look at their type (e2e/journey.spec.ts).
if [ -n "${THRUST_ASSETS_DIR:-}" ]; then
  # absolute: this script is in the Worker's directory by now, so a relative path would mean a different place than the caller's
  case "$THRUST_ASSETS_DIR" in
    /*) ;;
    *) echo "worker-dev: THRUST_ASSETS_DIR must be an absolute path, not '$THRUST_ASSETS_DIR'" >&2; exit 2 ;;
  esac
  [ -f "$THRUST_ASSETS_DIR/index.html" ] ||
    { echo "worker-dev: THRUST_ASSETS_DIR=$THRUST_ASSETS_DIR has no index.html (pnpm -C web --filter web build)" >&2; exit 2; }
  ASSETS_DIR="$(cd "$THRUST_ASSETS_DIR" && pwd)"
  if [ -z "$RUN_CONFIG" ]; then
    RUN_CONFIG=".wrangler.run-$$.toml"
    cp "$CONFIG" "$RUN_CONFIG"
    CONFIG="$RUN_CONFIG"
  fi
  {
    printf '\n[assets]\n'
    printf 'directory = "%s"\n' "$ASSETS_DIR"
    printf 'not_found_handling = "single-page-application"\n'
    printf 'run_worker_first = ["/api", "/api/*", "/auth", "/auth/*", "/live", "/live/*", "/healthz", "/_info", "/internal", "/internal/*", "/_test/*"]\n'
  } >>"$RUN_CONFIG"
  echo "worker-dev: serving the files of $ASSETS_DIR from $CONFIG"
fi

# Optional vars are passed only when set, so an unset one leaves the Worker's own default in force.
set --
for v in THRUST_ARGON2_M_KIB THRUST_ARGON2_T THRUST_ARGON2_P THRUST_SIGNIN_RATE_LIMIT THRUST_V1_RATE_LIMIT; do
  eval "val=\${$v:-}"
  [ -n "$val" ] && set -- "$@" --var "$v:$val"
done
# CSRF enforcement: on unless told otherwise; `default` passes nothing (the config's `[vars]` decide)
case "${THRUST_CSRF_ENFORCE:-on}" in
  default) ;;
  *) set -- "$@" --var "THRUST_CSRF_ENFORCE:${THRUST_CSRF_ENFORCE:-on}" ;;
esac

# Not `exec`: this shell has to outlive wrangler to remove the throwaway config (the status it returns is wrangler's).
env -i \
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
