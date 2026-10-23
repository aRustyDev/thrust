#!/bin/sh
# Run the ONE contract suite against one target.
#
#   scripts/contract.sh <target> [pass]
#     target: native-memory | native-pg | worker-memory | worker-cf
#     pass:   main (default)   normal config, outbox fast path ON
#             sweeper          fast path OFF; asserts the cron sweeper does the delivery
#             panic            alone against a fresh target (a Worker panic resets the isolate)
#             blast            OBSERVATIONAL, not in contract-all: failures of OTHER requests around one panic
#             bench            latency bench (release native build); success == the bench exits 0
#             worker         (worker-cf only) Durable Object WebSocket hibernation + SQLite
#             cron             (worker-* only) fast path OFF; the Worker `scheduled` handler sweeps
#             bindings         (worker-cf only) one real operation per binding (rate limit, secrets
#                              store, email, service binding, workflow via JS shim)
#
# Exit status is the status of `cargo test` itself (no pipes, no trailing echo). The suite's
# pass count is also checked against the expected number so that a filter that selects zero
# tests cannot report green.
#
# Secrets: the admin token and the throwaway Postgres password are generated per run and only
# ever live in this process's environment (and the children's), never in a file or on argv.
#
# Hygiene: only processes this script started are signalled (tree-kill by PID). Never `pkill`
# by name: other sessions on this machine run their own wrangler/workerd.
set -u

TARGET="${1:?usage: contract.sh <native-memory|native-pg|worker-memory|worker-cf> [main|sweeper|panic]}"
PASS="${2:-main}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOGDIR="${THRUST_LOGDIR:-$ROOT/target/contract-logs}"
mkdir -p "$LOGDIR"
LOG="$LOGDIR/$TARGET-$PASS"
RATE_LIMIT=30
# Passwords are hashed at a cheap argon2id cost on every local target (one hash is about a millisecond), because the
# suite signs in many times; OWASP's real cost is pinned by a core test and measured on a deployed Worker (issue #38).
# The sign-in limit is high enough that no test comes near it.
ARGON2_M_KIB=64
ARGON2_T=1
ARGON2_P=1
SIGNIN_LIMIT=30
EXPECT_MAIN=36
EXPECT_SWEEPER=2
EXPECT_PANIC=1
EXPECT_BLAST=1
EXPECT_WORKER=1
EXPECT_CRON=1
EXPECT_BINDINGS=6

# A free port, chosen at random: fixed ports collide with OrbStack-published containers and
# with other sessions' wrangler instances (measured: 18080 and 8787 were both taken).
pick_port() {
  n=0
  while [ "$n" -lt 50 ]; do
    p=$((20000 + $(od -An -N2 -tu2 /dev/urandom | tr -d ' ') % 30000))
    if ! lsof -nP -iTCP:"$p" -sTCP:LISTEN >/dev/null 2>&1; then
      echo "$p"
      return 0
    fi
    n=$((n + 1))
  done
  return 1
}

PORT="${THRUST_PORT:-$(pick_port)}"
THRUST_ADMIN_TOKEN="$(openssl rand -hex 16)"
export THRUST_ADMIN_TOKEN
export THRUST_BASE_URL="http://127.0.0.1:$PORT"
export THRUST_EXPECT_CREATE_LIMIT="$RATE_LIMIT"
export THRUST_ARGON2_M_KIB="$ARGON2_M_KIB" THRUST_ARGON2_T="$ARGON2_T" THRUST_ARGON2_P="$ARGON2_P"
export THRUST_SIGNIN_RATE_LIMIT="$SIGNIN_LIMIT"
# The mock OIDC identity provider: the suite serves it on 127.0.0.1:$THRUST_MOCK_IDP_PORT from the test process, and the
# target is configured to trust it. Every run has its own client secret, which only ever lives in this process's
# environment and its children's (the Worker gets it through `env -i` in worker-dev.sh, not on argv). A deployed Worker
# cannot reach 127.0.0.1, so scripts/stage2-contract.sh skips the `oidc_` tests.
THRUST_MOCK_IDP_PORT="$(pick_port)"
THRUST_OIDC_CLIENT_SECRET="$(openssl rand -hex 16)"
export THRUST_MOCK_IDP_PORT THRUST_OIDC_CLIENT_SECRET
export THRUST_OIDC_ISSUER="http://127.0.0.1:$THRUST_MOCK_IDP_PORT"
export THRUST_OIDC_CLIENT_ID="thrust-contract"
export THRUST_OIDC_REDIRECT_URI="$THRUST_BASE_URL/api/_slice/auth/oidc/callback"

case "$PASS" in
  sweeper|cron) FASTPATH=off ;;
  *) FASTPATH=on ;;
esac

SERVER_PID=""
PG_CONTAINER=""

kill_tree() {
  for child in $(pgrep -P "$1" 2>/dev/null); do
    kill_tree "$child"
  done
  kill -TERM "$1" 2>/dev/null || true
}

cleanup() {
  if [ -n "$SERVER_PID" ]; then
    kill_tree "$SERVER_PID"
    sleep 2
    kill -KILL "$SERVER_PID" 2>/dev/null || true
  fi
  if [ -n "$PG_CONTAINER" ]; then
    docker rm -f "$PG_CONTAINER" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

# native-pg  = self-hosted set: Postgres (deadpool) + in-process stand-ins for queue/KV/blob/limiter
# worker-cf  = Cloudflare set: Postgres via Hyperdrive, Queues, KV, R2, Durable Objects, cron
case "$TARGET" in
  native-memory) KIND=native; ADAPTERS=memory ;;
  native-pg) KIND=native; ADAPTERS=postgres ;;
  worker-memory) KIND=worker; ADAPTERS=memory ;;
  worker-cf) KIND=worker; ADAPTERS=cloudflare ;;
  *) echo "unknown target $TARGET" >&2; exit 2 ;;
esac

if [ "$ADAPTERS" = "postgres" ] || [ "$ADAPTERS" = "cloudflare" ]; then
  PG_PORT="${THRUST_PG_PORT:-$(pick_port)}"
  PG_PASSWORD="$(openssl rand -hex 12)"
  PG_CONTAINER="thrust-spike-pg-$$"
  # `-e NAME` without a value makes the docker CLI forward NAME from ITS environment, so the
  # password never appears on argv.
  POSTGRES_PASSWORD="$PG_PASSWORD" docker run -d --rm --name "$PG_CONTAINER" -e POSTGRES_PASSWORD -e POSTGRES_DB=thrust \
    -p "127.0.0.1:$PG_PORT:5432" postgres:16-alpine >"$LOG.pg.log" 2>&1 || { echo "could not start postgres" >&2; exit 3; }
  i=0
  until [ "$(docker exec "$PG_CONTAINER" psql -U postgres -d thrust -tAc 'select 1' 2>/dev/null)" = "1" ]; do
    i=$((i + 1)); [ "$i" -gt 60 ] && { echo "postgres query probe failed" >&2; exit 3; }
    sleep 1
  done
  # the official image restarts once after init; make sure the second start is the one we use
  sleep 3
  until [ "$(docker exec "$PG_CONTAINER" psql -U postgres -d thrust -tAc 'select 1' 2>/dev/null)" = "1" ]; do
    i=$((i + 1)); [ "$i" -gt 60 ] && { echo "postgres query probe failed (2)" >&2; exit 3; }
    sleep 1
  done
  export DATABASE_URL="postgres://postgres:$PG_PASSWORD@127.0.0.1:$PG_PORT/thrust"
  # Apply the schema through the very same SQL the adapter ships (no hand-copied DDL).
  "$ROOT/scripts/rs" cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p thrust-adapters-postgres --features native --bin migrate \
    >"$LOG.migrate.log" 2>&1 || { echo "migrate failed, see $LOG.migrate.log" >&2; exit 3; }
fi

if [ "$KIND" = "native" ]; then
  # THRUST_SERVER_FEATURES is only ever set by scripts/mutants.sh (control runs with broken builds).
  # The bench pass measures a release build; every other pass uses the debug build.
  if [ "$PASS" = "bench" ]; then PROFILE_FLAG="--release"; BIN_DIR=release; else PROFILE_FLAG=""; BIN_DIR=debug; fi
  # shellcheck disable=SC2086
  "$ROOT/scripts/rs" cargo build --quiet $PROFILE_FLAG --manifest-path "$ROOT/Cargo.toml" -p thrust-server ${THRUST_SERVER_FEATURES:+--features "$THRUST_SERVER_FEATURES"} \
    >"$LOG.build.log" 2>&1 || { echo "server build failed, see $LOG.build.log" >&2; exit 3; }
  THRUST_LISTEN="127.0.0.1:$PORT" THRUST_ADAPTERS="$ADAPTERS" THRUST_TEST_ROUTES=1 \
    THRUST_CREATE_RATE_LIMIT="$RATE_LIMIT" THRUST_OUTBOX_FASTPATH="$FASTPATH" THRUST_SWEEP_INTERVAL_SECS=0 \
    "$ROOT/target/$BIN_DIR/thrust-server" >"$LOG.server.log" 2>&1 &
  SERVER_PID=$!
else
  # Worker: `wrangler dev` (local workerd). See scripts/worker-dev.sh.
  THRUST_PORT="$PORT" THRUST_ADAPTERS="$ADAPTERS" THRUST_OUTBOX_FASTPATH="$FASTPATH" THRUST_CREATE_RATE_LIMIT="$RATE_LIMIT" \
    "$ROOT/scripts/worker-dev.sh" >"$LOG.server.log" 2>&1 &
  SERVER_PID=$!
fi

i=0
until curl -fsS "$THRUST_BASE_URL/healthz" >/dev/null 2>&1; do
  i=$((i + 1))
  if [ "$i" -gt 240 ]; then
    echo "target did not become healthy; tail of server log:" >&2
    tail -30 "$LOG.server.log" >&2
    exit 4
  fi
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "target process exited early; tail of server log:" >&2
    tail -30 "$LOG.server.log" >&2
    exit 4
  fi
  sleep 1
done
echo "[$TARGET/$PASS] target healthy at $THRUST_BASE_URL (adapters=$ADAPTERS fastpath=$FASTPATH)"

# Vacuity guard: ask the target which adapter set it is REALLY running and compare with what this
# run claims to be testing. (A mislabelled run that exercised the wrong set would otherwise pass.)
# A few retries: right after startup a `wrangler dev` worker can still be reloading (measured: one
# `worker-cf blast` run got an empty reply here although /healthz had just answered). The guard
# still fails closed if no answer ever arrives.
INFO=""
i=0
while [ "$i" -lt 8 ]; do
  INFO="$(curl -fsS -H "x-admin-token: $THRUST_ADMIN_TOKEN" "$THRUST_BASE_URL/_info" 2>/dev/null || true)"
  [ -n "$INFO" ] && break
  i=$((i + 1))
  sleep 1
done
echo "[$TARGET/$PASS] /_info: $INFO"
case "$INFO" in
  *"\"target\":\"$KIND\""*) ;;
  *) echo "[$TARGET/$PASS] VACUITY GUARD: target kind is not '$KIND'" >&2; exit 5 ;;
esac
case "$INFO" in
  *"\"adapters\":\"$ADAPTERS\""*) ;;
  *) echo "[$TARGET/$PASS] VACUITY GUARD: adapter set is not '$ADAPTERS'" >&2; exit 5 ;;
esac

if [ "$TARGET" = "worker-memory" ]; then
  # state lives inside the isolate; a panic (abort) resets it
  THRUST_STATE_SURVIVES_PANIC=0
else
  THRUST_STATE_SURVIVES_PANIC=1
fi
export THRUST_STATE_SURVIVES_PANIC

case "$PASS" in
  sweeper)
    "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test contract -- sweeper_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    EXPECT=$EXPECT_SWEEPER
    ;;
  panic)
    "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test contract -- panic_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    EXPECT=$EXPECT_PANIC
    ;;
  blast)
    # OBSERVATIONAL (not in contract-all): panics the handler while 16 clients run an I/O-bound
    # request and reports how many OTHER requests failed. Under `wrangler dev` the memory set can
    # lose the whole dev server here, which is itself an observation, so this is run by hand.
    "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test contract -- blast_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    EXPECT=$EXPECT_BLAST
    ;;
  worker)
    "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test worker_only -- worker_room_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    EXPECT=$EXPECT_WORKER
    ;;
  bench)
    # Release-built bench client; the target under test is whatever this script just started.
    # Not a test: success == the bench binary exiting 0. Numbers are LOCAL (loopback), see research/0004.
    { "$ROOT/scripts/rs" cargo run --quiet --release --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --bin bench -- 200 1 \
        && "$ROOT/scripts/rs" cargo run --quiet --release --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --bin bench -- 200 8; } >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    cat "$LOG.test.log"
    echo "[$TARGET/$PASS] bench exit=$TEST_STATUS"
    exit "$TEST_STATUS"
    ;;
  bindings)
    "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test worker_only -- worker_bindings_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    EXPECT=$EXPECT_BINDINGS
    ;;
  cron)
    "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test worker_only -- worker_cron_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    EXPECT=$EXPECT_CRON
    ;;
  *)
    "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test contract -- --skip sweeper_ --skip panic_ --skip blast_ >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    EXPECT=$EXPECT_MAIN
    ;;
esac

grep -E "PANIC-OBSERVATION|HIBERNATION-OBSERVATION" "$LOG.test.log" || true

grep -E "^test |^test result|panicked|FAILED" "$LOG.test.log"
PASSED="$(sed -n 's/^test result: [a-zA-Z]*\. \([0-9]*\) passed.*/\1/p' "$LOG.test.log" | tail -1)"
echo "[$TARGET/$PASS] cargo test exit=$TEST_STATUS passed=${PASSED:-?} expected=$EXPECT"
if [ "$TEST_STATUS" -eq 0 ] && [ "${PASSED:-0}" -ne "$EXPECT" ]; then
  echo "[$TARGET/$PASS] VACUITY GUARD: exit 0 but passed count != expected" >&2
  exit 5
fi
exit "$TEST_STATUS"
