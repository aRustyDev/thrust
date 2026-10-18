#!/bin/sh
# Run the ONE contract suite against one target.
#
#   scripts/contract.sh <target> [pass]
#     target: native-memory | native-pg | worker-memory | worker-pg
#     pass:   main (default)   normal config, outbox fast path ON
#             sweeper          fast path OFF; asserts the cron sweeper does the delivery
#
# Exit status is the status of `cargo test` itself (no pipes, no trailing echo). The suite's
# pass count is also checked against the expected number so that a filter that selects zero
# tests cannot report green.
#
# Secrets: the admin token is generated per run and only ever lives in this process's
# environment (and the child processes' environment), never in a file or on argv.
set -u

TARGET="${1:?usage: contract.sh <native-memory|native-pg|worker-memory|worker-pg> [main|sweeper]}"
PASS="${2:-main}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOGDIR="${THRUST_LOGDIR:-$ROOT/target/contract-logs}"
mkdir -p "$LOGDIR"
LOG="$LOGDIR/$TARGET-$PASS"
PORT="${THRUST_PORT:-18080}"
RATE_LIMIT=30
EXPECT_MAIN=15
EXPECT_SWEEPER=1

THRUST_ADMIN_TOKEN="$(openssl rand -hex 16)"
export THRUST_ADMIN_TOKEN
export THRUST_BASE_URL="http://127.0.0.1:$PORT"
export THRUST_EXPECT_CREATE_LIMIT="$RATE_LIMIT"

if [ "$PASS" = "sweeper" ]; then
  FASTPATH=off
else
  FASTPATH=on
fi

SERVER_PID=""
PG_CONTAINER=""
cleanup() {
  if [ -n "$SERVER_PID" ]; then
    # kill the whole process group so `wrangler dev` children (workerd) die with it
    kill -TERM "-$SERVER_PID" 2>/dev/null || kill -TERM "$SERVER_PID" 2>/dev/null
    sleep 1
    kill -KILL "-$SERVER_PID" 2>/dev/null || true
  fi
  if [ -n "$PG_CONTAINER" ]; then
    docker rm -f "$PG_CONTAINER" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

case "$TARGET" in
  native-*) KIND=native ;;
  worker-*) KIND=worker ;;
  *) echo "unknown target $TARGET" >&2; exit 2 ;;
esac
case "$TARGET" in
  *-pg) ADAPTERS=postgres ;;
  *-memory) ADAPTERS=memory ;;
  *) echo "unknown target $TARGET" >&2; exit 2 ;;
esac

if [ "$ADAPTERS" = "postgres" ]; then
  PG_PORT="${THRUST_PG_PORT:-15432}"
  PG_PASSWORD="$(openssl rand -hex 12)"
  PG_CONTAINER="thrust-spike-pg-$$"
  docker run -d --rm --name "$PG_CONTAINER" -e POSTGRES_PASSWORD="$PG_PASSWORD" -e POSTGRES_DB=thrust \
    -p "127.0.0.1:$PG_PORT:5432" postgres:16-alpine >"$LOG.pg.log" 2>&1 || { echo "could not start postgres" >&2; exit 3; }
  i=0
  until docker exec "$PG_CONTAINER" pg_isready -U postgres -d thrust >/dev/null 2>&1; do
    i=$((i + 1)); [ "$i" -gt 60 ] && { echo "postgres did not become ready" >&2; exit 3; }
    sleep 1
  done
  # pg_isready can flip before the init-time restart completes; wait for a real query to succeed twice
  i=0
  until [ "$(docker exec "$PG_CONTAINER" psql -U postgres -d thrust -tAc 'select 1' 2>/dev/null)" = "1" ]; do
    i=$((i + 1)); [ "$i" -gt 30 ] && { echo "postgres query probe failed" >&2; exit 3; }
    sleep 1
  done
  sleep 2
  export DATABASE_URL="postgres://postgres:$PG_PASSWORD@127.0.0.1:$PG_PORT/thrust"
  # Apply the schema through the very same SQL the adapter ships (no hand-copied DDL).
  "$ROOT/scripts/rs" cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p thrust-adapters-postgres --bin migrate \
    >"$LOG.migrate.log" 2>&1 || { echo "migrate failed, see $LOG.migrate.log" >&2; exit 3; }
fi

if [ "$KIND" = "native" ]; then
  "$ROOT/scripts/rs" cargo build --quiet --manifest-path "$ROOT/Cargo.toml" -p thrust-server ${THRUST_SERVER_FEATURES:+--features "$THRUST_SERVER_FEATURES"} \
    >"$LOG.build.log" 2>&1 || { echo "server build failed, see $LOG.build.log" >&2; exit 3; }
  set -m
  THRUST_LISTEN="127.0.0.1:$PORT" THRUST_ADAPTERS="$ADAPTERS" THRUST_TEST_ROUTES=1 \
    THRUST_CREATE_RATE_LIMIT="$RATE_LIMIT" THRUST_OUTBOX_FASTPATH="$FASTPATH" THRUST_SWEEP_INTERVAL_SECS=0 \
    "$ROOT/target/debug/thrust-server" >"$LOG.server.log" 2>&1 &
  SERVER_PID=$!
else
  # Worker: built bundle + `wrangler dev` (local workerd). See scripts/worker-dev.sh.
  set -m
  THRUST_PORT="$PORT" THRUST_ADAPTERS="$ADAPTERS" THRUST_OUTBOX_FASTPATH="$FASTPATH" THRUST_CREATE_RATE_LIMIT="$RATE_LIMIT" \
    "$ROOT/scripts/worker-dev.sh" >"$LOG.server.log" 2>&1 &
  SERVER_PID=$!
fi

i=0
until curl -fsS "$THRUST_BASE_URL/healthz" >/dev/null 2>&1; do
  i=$((i + 1))
  if [ "$i" -gt 180 ]; then
    echo "target did not become healthy; tail of server log:" >&2
    tail -30 "$LOG.server.log" >&2
    exit 4
  fi
  sleep 1
done
echo "[$TARGET/$PASS] target healthy at $THRUST_BASE_URL (adapters=$ADAPTERS fastpath=$FASTPATH)"

if [ "$PASS" = "sweeper" ]; then
  "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test contract -- sweeper_ --test-threads=1 >"$LOG.test.log" 2>&1
  TEST_STATUS=$?
  EXPECT=$EXPECT_SWEEPER
else
  "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test contract -- --skip sweeper_ >"$LOG.test.log" 2>&1
  TEST_STATUS=$?
  EXPECT=$EXPECT_MAIN
fi

grep -E "^test |^test result|panicked|FAILED" "$LOG.test.log"
PASSED="$(sed -n 's/^test result: [a-zA-Z]*\. \([0-9]*\) passed.*/\1/p' "$LOG.test.log" | tail -1)"
echo "[$TARGET/$PASS] cargo test exit=$TEST_STATUS passed=${PASSED:-?} expected=$EXPECT"
if [ "$TEST_STATUS" -eq 0 ] && [ "${PASSED:-0}" -ne "$EXPECT" ]; then
  echo "[$TARGET/$PASS] VACUITY GUARD: exit 0 but passed count != expected" >&2
  exit 5
fi
exit "$TEST_STATUS"
