#!/bin/sh
# Run the ONE contract suite against one target.
#
#   scripts/contract.sh <target> [pass]
#     target: native-memory | native-pg | worker-memory | worker-pg
#     pass:   main (default)   normal config, outbox fast path ON
#             sweeper          fast path OFF; asserts the cron sweeper does the delivery
#             panic            alone against a fresh target (a Worker panic resets the isolate)
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

TARGET="${1:?usage: contract.sh <native-memory|native-pg|worker-memory|worker-pg> [main|sweeper]}"
PASS="${2:-main}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOGDIR="${THRUST_LOGDIR:-$ROOT/target/contract-logs}"
mkdir -p "$LOGDIR"
LOG="$LOGDIR/$TARGET-$PASS"
RATE_LIMIT=30
EXPECT_MAIN=14
EXPECT_SWEEPER=1
EXPECT_PANIC=1

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

if [ "$PASS" = "sweeper" ]; then
  FASTPATH=off
else
  FASTPATH=on
fi

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
  "$ROOT/scripts/rs" cargo build --quiet --manifest-path "$ROOT/Cargo.toml" -p thrust-server \
    >"$LOG.build.log" 2>&1 || { echo "server build failed, see $LOG.build.log" >&2; exit 3; }
  THRUST_LISTEN="127.0.0.1:$PORT" THRUST_ADAPTERS="$ADAPTERS" THRUST_TEST_ROUTES=1 \
    THRUST_CREATE_RATE_LIMIT="$RATE_LIMIT" THRUST_OUTBOX_FASTPATH="$FASTPATH" THRUST_SWEEP_INTERVAL_SECS=0 \
    "$ROOT/target/debug/thrust-server" >"$LOG.server.log" 2>&1 &
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
  *)
    "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test contract -- --skip sweeper_ --skip panic_ >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    EXPECT=$EXPECT_MAIN
    ;;
esac

grep -E "PANIC-OBSERVATION" "$LOG.test.log" || true

grep -E "^test |^test result|panicked|FAILED" "$LOG.test.log"
PASSED="$(sed -n 's/^test result: [a-zA-Z]*\. \([0-9]*\) passed.*/\1/p' "$LOG.test.log" | tail -1)"
echo "[$TARGET/$PASS] cargo test exit=$TEST_STATUS passed=${PASSED:-?} expected=$EXPECT"
if [ "$TEST_STATUS" -eq 0 ] && [ "${PASSED:-0}" -ne "$EXPECT" ]; then
  echo "[$TARGET/$PASS] VACUITY GUARD: exit 0 but passed count != expected" >&2
  exit 5
fi
exit "$TEST_STATUS"
