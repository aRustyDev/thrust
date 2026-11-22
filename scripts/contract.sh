#!/bin/sh
# Run the ONE contract suite against one target.
#
#   scripts/contract.sh <target> [pass]
#     target: native-memory | native-pg | worker-memory | worker-cf
#     pass:   main (default)   normal config, outbox fast path ON
#             sweeper          fast path OFF; asserts the cron sweeper does the delivery
#             csrfoff          CSRF enforcement OFF (Q-0050.8, the production default while the stock frontend is in use);
#                              asserts a cookie mutation is accepted without the token. The target is started with NO
#                              setting for it, so the pass also shows that the composition root's own default is off
#                              (the native server's, and the Worker's `[vars]`). Every other pass runs with it ON
#             panic            alone against a fresh target (a Worker panic resets the isolate)
#             blast            OBSERVATIONAL, not in contract-all: failures of OTHER requests around one panic
#             bench            the latency load tests (thrust-load-tests' load_latency_*, release-built); success == every request
#                              succeeded (nextest exit 0)
#             scale            the scale load test (load_scale_*): list, search and create latency with THRUST_SCALE_TOTAL (default
#                              100000) work items in the database, in projects of THRUST_SCALE_PER_PROJECT (default 1000) (spike
#                              0002); like bench, success == every request succeeded
#             worker         (worker-cf only) Durable Object WebSocket hibernation + SQLite
#             cron             (worker-* only) fast path OFF; the Worker `scheduled` handler sweeps
#             engine           the tests that decide the database engine (spike 0002): the 50-writer, 10k-create test,
#                              and the search tests. Own test binary (`--test engine`), so EXPECT_MAIN is untouched.
#                              The target starts with a very high `/api/v1` allowance per key. Opt-in: not in contract-all.sh.
#             bindings         (worker-cf only) the probe routes' admin gate, and one real operation per binding (rate limit,
#                              secrets store, email)
#             e2e              (worker-* only) the first browser journey (issue #44): the stock app, built from web/, in Chromium against
#                              the Worker started here, which serves the app's files (scripts/worker-dev.sh THRUST_ASSETS_DIR) with CSRF
#                              enforcement at the Worker's own default, off, as production runs it while the stock app is in use. Needs
#                              the app built (web/apps/web/build/client, or E2E_ASSETS_DIR) and node and pnpm; success == its
#                              `VERDICT: PASS`, with its must-fail control unless E2E_WHAT is set to '' for none. In contract-all.sh, so a
#                              CI cell. scripts/e2e.sh <target> [controls] is the way in by hand; its logs, and the network's HAR, go to
#                              target/contract-logs/<target>-e2e.e2e/
#             e2e-mail         (worker-* only) the browser journeys that need a mail service (e2e/journeys-mail/, E2E_SUITE=mail of
#                              scripts/e2e.sh): the e2e pass's Worker and app, with the `mail` pass's mail service (the `send_email`
#                              binding as wrangler dev simulates it, whose log the journey reads for the code it is mailed) and
#                              THRUST_PUBLIC_BASE_URL, so the app draws "Sign in with unique code". A pass of its own because the e2e
#                              pass holds that the target has NO mail (the sign-in journey asserts the button is absent, and the
#                              pinned Plane has no mail either), and its count guard holds every journey of e2e/journeys/ to that. In
#                              contract-all.sh, so a CI cell; scripts/e2e.sh <target> mail [controls] is the way in by hand; its logs go
#                              to target/contract-logs/<target>-e2e-mail.e2e/
#             mail             the password reset by email on a target that has a mail service (D-041), and `--test mail` walks the whole
#                              path: the request, the message the mail service was given, the link in it, and the reset. A native target
#                              (E2) gets THRUST_SMTP_* pointing at a Mailpit container (Docker) started beside it, whose API is where the
#                              mail is read; a Worker target (E3) gets THRUST_EMAIL_FROM, its `send_email` binding being `wrangler dev`'s
#                              simulator, whose log (and files) are where the mail is read, so that the Worker's own adapter, built for
#                              wasm32, is the one that is run. Both get THRUST_PUBLIC_BASE_URL. Every other pass starts its target with no
#                              mail service, as the deployments are, and holds that the route is dark there. In contract-all.sh, so a CI cell
#             oracle           the v1 oracle's scoped gate and corpus comparison against the started target, with the /api/v1 rate
#                              limit raised for the fuzzer, and (unless ORACLE_V1_WHAT is set, to '' for none) its must-fail
#                              controls. In contract-all.sh, so a CI cell; needs `uv`. Success == its `VERDICT: PASS`, and its
#                              logs go to target/contract-logs/<target>-oracle.oracle/. scripts/oracle-v1.sh <target> [controls]
#                              is the way in by hand, and runs the controls only when asked
#
# Each pass runs on nextest: `cargo nextest run -p thrust-contract-tests --profile contract -E 'binary(<bin>) & (<filterset>)'`
# (the `contract` profile is in .config/nextest.toml). Exit status is nextest's own (no pipes, no trailing echo): 100 when a
# test failed. The pass's passed count, read from nextest's JUnit report (kept beside the pass's log as `<log>.junit.xml`),
# is also checked against the expected number, so that a filterset that selects too few tests, or none, cannot report
# green: either is exit 5. (nextest's own 4, "no test ran", becomes 5 too, because this script's 4 means the target never
# came up.)
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
# What a key may do on /api/v1 in a minute: Plane's default, which the target applies when THRUST_V1_RATE_LIMIT is not set. The suite is
# told it as THRUST_EXPECT_V1_LIMIT. A control run that starts the target with another
# THRUST_V1_RATE_LIMIT in the environment, and not this script, makes `v1_requests_are_rate_limited_per_key` fail: the target then
# lets more through than the suite expects.
V1_LIMIT=60
# The engine pass makes thousands of `/api/v1` requests per key (every create goes through `/api/v1`): Plane's default of 60 a minute
# would refuse them.
if [ "${2:-main}" = "engine" ]; then
  V1_LIMIT=1000000
  export THRUST_V1_RATE_LIMIT=1000000/minute
fi
# The oracle pass fuzzes /api/v1 with one key: Plane's default of 60 requests a minute would answer nearly all of them 429. The
# other passes run with the default, as production does.
if [ "${2:-main}" = "oracle" ]; then
  export THRUST_V1_RATE_LIMIT=1000000/minute
  V1_LIMIT=1000000
fi
# ... and 50 parallel writers hold up to ~100 Postgres connections at once on the Worker path (each request opens one for
# its reads and one for its transaction; wrangler's local Hyperdrive emulation does not pool, a deployed Hyperdrive
# does), which is the image's default `max_connections`. Measured: at 100, 1 of 50 writers got HTTP 500 ("postgres
# handshake: db error", the server's reason is not in the Worker's log) in 1 run; at 400, 0 in 2 runs. The connection
# limit is the inferred cause.
PG_MAX_CONNECTIONS=100
[ "${2:-main}" = "engine" ] && PG_MAX_CONNECTIONS=400
# Passwords are hashed at a cheap argon2id cost on every local target (one hash is about a millisecond), because the
# suite signs in many times; OWASP's real cost is pinned by a core test and measured on a deployed Worker (issue #38).
# The sign-in limit is high enough that no test comes near it.
ARGON2_M_KIB=64
ARGON2_T=1
ARGON2_P=1
SIGNIN_LIMIT=30
EXPECT_MAIN=75
EXPECT_BUDGET=1
EXPECT_SWEEPER=4
EXPECT_CSRFOFF=6
EXPECT_PANIC=1
EXPECT_BLAST=1
EXPECT_WORKER=1
EXPECT_CRON=2
EXPECT_BINDINGS=6
EXPECT_ENGINE=4
EXPECT_MAIL=7
# The mail pass's relay: Mailpit, pinned (the SMTP adapter's own tests, tests/smtp_sink, pin the same version).
MAILPIT_IMAGE=axllent/mailpit:v1.33.1

# Every port this run binds is drawn by `pick_port`, from below the kernel's outgoing-connection range and never twice (why: the
# library). The run's ports are kept in $LOG.ports, one a line in the order they were drawn, with the logs.
# shellcheck source=scripts/lib/ports.sh
. "$ROOT/scripts/lib/ports.sh"
PORTS_DRAWN_FILE="$LOG.ports"
: >"$PORTS_DRAWN_FILE"
ports_remember "${THRUST_PORT:-}" "${THRUST_MOCK_IDP_PORT:-}" "${THRUST_PG_PORT:-}" "${THRUST_MAIL_SMTP_PORT:-}" \
  "${THRUST_MAIL_HTTP_PORT:-}" || exit 3
PORTS_TOP="$(ports_top)" || exit 3
echo "[$TARGET/$PASS] ports drawn from $PORTS_FLOOR-$PORTS_TOP (the kernel's outgoing range starts at $(ephemeral_low))"

PORT="${THRUST_PORT:-}"
[ -n "$PORT" ] || PORT="$(pick_port)" || exit 3
THRUST_ADMIN_TOKEN="$(openssl rand -hex 16)"
export THRUST_ADMIN_TOKEN
# The operator's routes (/internal/operator/*, Q-F7) are a door of their own, with a token of their own: the target is started with
# them on, as the tests of them need, and the test routes' token opens nothing there (a contract test says so). 32 hex digits is
# the shortest token the door accepts.
THRUST_OPERATOR_TOKEN="$(openssl rand -hex 16)"
export THRUST_OPERATOR_TOKEN
export THRUST_OPERATOR_ROUTES=1
export THRUST_BASE_URL="http://127.0.0.1:$PORT"
export THRUST_EXPECT_V1_LIMIT="$V1_LIMIT"
export THRUST_ARGON2_M_KIB="$ARGON2_M_KIB" THRUST_ARGON2_T="$ARGON2_T" THRUST_ARGON2_P="$ARGON2_P"
export THRUST_SIGNIN_RATE_LIMIT="$SIGNIN_LIMIT"
# The mock OIDC identity provider: the suite serves it on 127.0.0.1:$THRUST_MOCK_IDP_PORT from the test process, and the
# target is configured to trust it. Every run has its own client secret, which only ever lives in this process's
# environment and its children's (the Worker gets it through `env -i` in worker-dev.sh, not on argv). A deployed Worker
# cannot reach 127.0.0.1, so scripts/stage2-contract.sh skips the `oidc_` tests.
THRUST_MOCK_IDP_PORT="${THRUST_MOCK_IDP_PORT:-}"
[ -n "$THRUST_MOCK_IDP_PORT" ] || THRUST_MOCK_IDP_PORT="$(pick_port)" || exit 3
THRUST_OIDC_CLIENT_SECRET="$(openssl rand -hex 16)"
export THRUST_MOCK_IDP_PORT THRUST_OIDC_CLIENT_SECRET
export THRUST_OIDC_ISSUER="http://127.0.0.1:$THRUST_MOCK_IDP_PORT"
export THRUST_OIDC_CLIENT_ID="thrust-contract"
# The redirect URI is the stock app's callback, the one a deployment registers at its provider (D-040: `<origin>/auth/oidc/callback/`).
export THRUST_OIDC_REDIRECT_URI="$THRUST_BASE_URL/auth/oidc/callback/"
# The e2e pass is the stock app in a browser, and the app draws an SSO button when `/api/instances/` says a provider is configured (D-040). The deployed
# Worker has none (it cannot reach a WARP-only issuer until Q-0050.9), and the sign-in journey holds that the page shows the password and nothing else, so
# that pass starts its target as the deployment runs: with no provider. (worker-dev.sh passes an empty setting on, which is none.)
if [ "$PASS" = e2e ] || [ "$PASS" = e2e-mail ]; then
  unset THRUST_OIDC_ISSUER THRUST_OIDC_CLIENT_ID THRUST_OIDC_CLIENT_SECRET THRUST_OIDC_REDIRECT_URI
fi
# Mail: only the `mail` pass has a mail service. Every other pass starts its target the way the deployments are today, with none, and the main
# pass holds that the route is dark there (`compat_forgot_password_is_dark_on_a_target_with_no_mail_service`), so a developer's own THRUST_SMTP_*
# or THRUST_PUBLIC_BASE_URL in the shell must not reach it. The `mail` pass sets its own below, once its relay is up. THRUST_SMTP_PASSWORD is a
# secret and stays out of the environment of any pass that does not use one.
unset THRUST_SMTP_HOST THRUST_SMTP_PORT THRUST_SMTP_TLS THRUST_SMTP_USERNAME THRUST_SMTP_PASSWORD THRUST_SMTP_FROM THRUST_SMTP_CA_FILE \
  THRUST_SMTP_TIMEOUT_SECS THRUST_PUBLIC_BASE_URL THRUST_MAIL_API_URL THRUST_EMAIL_FROM THRUST_EMAIL_FROM_NAME THRUST_MAIL_WRANGLER_LOG

case "$PASS" in
  sweeper|cron) FASTPATH=off ;;
  *) FASTPATH=on ;;
esac

# The retention windows (issue #59): a developer's own must not reach the target, so every pass runs at the defaults (30 days for a work
# item, 90 for the outbox), which the `sweeper` pass reads back from /_info. The exception is `cron`: its test of the Worker's `scheduled`
# handler needs a work item that was deleted a moment ago to be old enough for the hard delete, so that pass has a window of no days and
# the test refuses a target that says otherwise. No other test of that pass deletes anything.
unset THRUST_HARD_DELETE_AFTER_DAYS THRUST_OUTBOX_RETENTION_DAYS
if [ "$PASS" = cron ]; then export THRUST_HARD_DELETE_AFTER_DAYS=0; fi

# CSRF enforcement (Q-0050.8). The composition roots default it OFF, but the contract targets keep the meaning of their CSRF
# tests, so every pass runs with it ON except `csrfoff`, which starts the target with no setting at all ("default": the
# native server gets no THRUST_CSRF_ENFORCE, the Worker no `--var` for it, so its `[vars]` apply) and so shows the default.
# WANT_CSRF is what /_info must say; the vacuity guard below refuses a target that says otherwise.
# THRUST_SERVER_CSRF_ENFORCE (on|off) is only ever set by scripts/mutants.sh (control runs): it starts the server with a
# setting the pass does not expect, to show that the guard refuses it.
# The e2e pass is the stock app's own journey, which sends the token on its auth posts only, so it too runs at the default (off).
case "$PASS" in
  csrfoff | e2e | e2e-mail) CSRF=off; WANT_CSRF=false; CSRF_DEFAULT=default ;;
  *) CSRF=on; WANT_CSRF=true; CSRF_DEFAULT=on ;;
esac
CSRF_SERVER="${THRUST_SERVER_CSRF_ENFORCE:-$CSRF_DEFAULT}"
unset THRUST_CSRF_ENFORCE

SERVER_PID=""
PG_CONTAINER=""
MAIL_CONTAINER=""
PROXY_PID=""

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
  if [ -n "$MAIL_CONTAINER" ]; then
    docker rm -f "$MAIL_CONTAINER" >/dev/null 2>&1 || true
  fi
  if [ -n "$PROXY_PID" ]; then
    kill -TERM "$PROXY_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

# native-pg  = self-hosted set: Postgres (deadpool) + in-process stand-ins for queue/KV/blob/limiter
# worker-cf  = Cloudflare set: Postgres via Hyperdrive, Queues, R2, the Workers Rate Limiting bindings, Durable Objects, cron.
#              The sign-in and `/api/v1` limits are those bindings' limits, which scripts/worker-dev.sh writes from SIGNIN_LIMIT and
#              from THRUST_V1_RATE_LIMIT (`RL_SIGNIN` and `RL_V1`). The control for the key's-allowance test is
#              THRUST_V1_RATE_LIMIT=<above 2 x 60 + 1>/minute, set in the environment of this script (it is not the one the suite
#              is told, V1_LIMIT): the binding then lets more through than the suite expects, and
#              `v1_requests_are_rate_limited_per_key` must fail (issue #43).
case "$TARGET" in
  native-memory) KIND=native; ADAPTERS=memory ;;
  native-pg) KIND=native; ADAPTERS=postgres ;;
  worker-memory) KIND=worker; ADAPTERS=memory ;;
  worker-cf) KIND=worker; ADAPTERS=cloudflare ;;
  *) echo "unknown target $TARGET" >&2; exit 2 ;;
esac

if [ "$PASS" = "e2e" ] || [ "$PASS" = "e2e-mail" ]; then
  # Only a Worker serves the app (ADR-0011, round 29: the native server's static files are not built), and it must have the app to serve.
  [ "$KIND" = worker ] || { echo "[$TARGET/$PASS] the native server does not serve the app yet: the e2e pass is for the worker targets" >&2; exit 2; }
  E2E_ASSETS_DIR="${E2E_ASSETS_DIR:-$ROOT/web/apps/web/build/client}"
  [ -s "$E2E_ASSETS_DIR/index.html" ] ||
    { echo "[$TARGET/$PASS] the app is not built: $E2E_ASSETS_DIR/index.html is missing (scripts/e2e.sh $TARGET builds it)" >&2; exit 2; }
fi

if [ "$ADAPTERS" = "postgres" ] || [ "$ADAPTERS" = "cloudflare" ]; then
  PG_PORT="${THRUST_PG_PORT:-}"
  [ -n "$PG_PORT" ] || PG_PORT="$(pick_port)" || exit 3
  PG_PASSWORD="$(openssl rand -hex 12)"
  PG_CONTAINER="thrust-spike-pg-$$"
  # `-e NAME` without a value makes the docker CLI forward NAME from ITS environment, so the
  # password never appears on argv.
  POSTGRES_PASSWORD="$PG_PASSWORD" docker run -d --rm --name "$PG_CONTAINER" -e POSTGRES_PASSWORD -e POSTGRES_DB=thrust \
    -p "127.0.0.1:$PG_PORT:5432" postgres:16-alpine -c "max_connections=$PG_MAX_CONNECTIONS" >"$LOG.pg.log" 2>&1 ||
    { echo "could not start postgres (port $PG_PORT); docker said:" >&2; tail -5 "$LOG.pg.log" >&2; exit 3; }
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

if [ -n "${THRUST_PG_LATENCY_MS:-}" ] && [ -n "${PG_PORT:-}" ]; then
  # Put a fixed one-way delay between the server and Postgres (after the schema is applied directly), so a loopback run
  # pays something like the round trips of Hyperdrive + VPC service + tunnel. For the FR-22 before/after (research/0011).
  PROXY_PORT="$(pick_port)" || exit 3
  uv run --quiet "$ROOT/scripts/pg-delay-proxy.py" "$PROXY_PORT" "$PG_PORT" "$THRUST_PG_LATENCY_MS" >"$LOG.proxy.log" 2>&1 &
  PROXY_PID=$!
  sleep 2
  kill -0 "$PROXY_PID" 2>/dev/null || { echo "delay proxy did not start, see $LOG.proxy.log" >&2; exit 3; }
  export DATABASE_URL="postgres://postgres:$PG_PASSWORD@127.0.0.1:$PROXY_PORT/thrust"
  echo "[$TARGET/$PASS] postgres behind a ${THRUST_PG_LATENCY_MS} ms one-way delay proxy (port $PROXY_PORT)"
fi

if { [ "$PASS" = "mail" ] || [ "$PASS" = "e2e-mail" ]; } && [ "$KIND" != native ]; then
  # (the e2e-mail pass is a Worker's only, and reads the same log from a browser journey, e2e/support/mail.ts, as the mail pass's tests do)
  # A Worker has no relay: its mail service is the `send_email` binding, and under `wrangler dev` (local, no `remote = true`) the binding is a
  # simulator that sends nothing and writes each message it is asked to send to the dev server's log (the sender, the recipient, the subject)
  # and to files (the text and the html), by the paths the log gives. That is wrangler's own account of what the binding was given, so the tests
  # read it as they read Mailpit's API: from outside the adapter. The server's log is the file the Worker is started into below.
  export THRUST_EMAIL_FROM=no-reply@thrust.test THRUST_EMAIL_FROM_NAME=Thrust
  export THRUST_PUBLIC_BASE_URL="$THRUST_BASE_URL"
  export THRUST_MAIL_WRANGLER_LOG="$LOG.server.log"
  echo "[$TARGET/$PASS] mail service: the Worker's send_email binding as wrangler dev simulates it (read from $THRUST_MAIL_WRANGLER_LOG)"
fi

if [ "$PASS" = "mail" ] && [ "$KIND" = native ]; then
  # The relay: Mailpit, a mail server and sink written by someone else, so that what the server sends is read by a server that parses it for
  # itself. Plain (no TLS, no login: the adapter's own tests, tests/smtp_sink, are where those are shown). The native server's mail service is
  # its SMTP adapter; the Worker's is above.
  MAIL_SMTP_PORT="${THRUST_MAIL_SMTP_PORT:-}"
  [ -n "$MAIL_SMTP_PORT" ] || MAIL_SMTP_PORT="$(pick_port)" || exit 3
  MAIL_HTTP_PORT="${THRUST_MAIL_HTTP_PORT:-}"
  [ -n "$MAIL_HTTP_PORT" ] || MAIL_HTTP_PORT="$(pick_port)" || exit 3
  MAIL_CONTAINER="thrust-contract-mail-$$"
  docker run -d --rm --name "$MAIL_CONTAINER" -p "127.0.0.1:$MAIL_SMTP_PORT:1025" -p "127.0.0.1:$MAIL_HTTP_PORT:8025" "$MAILPIT_IMAGE" \
    >"$LOG.mail.log" 2>&1 ||
    { echo "could not start the mail server (ports $MAIL_SMTP_PORT, $MAIL_HTTP_PORT); docker said:" >&2; tail -5 "$LOG.mail.log" >&2; exit 3; }
  # up is what a client sees: the API answers and the SMTP port says 220
  i=0
  until curl -fsS "http://127.0.0.1:$MAIL_HTTP_PORT/api/v1/info" >/dev/null 2>&1 &&
    [ "$(printf 'QUIT\r\n' | nc -w 2 127.0.0.1 "$MAIL_SMTP_PORT" 2>/dev/null | head -n 1 | cut -c1-3)" = 220 ]; do
    i=$((i + 1)); [ "$i" -gt 60 ] && { echo "the mail server did not come up" >&2; exit 3; }
    sleep 1
  done
  export THRUST_SMTP_HOST=127.0.0.1 THRUST_SMTP_PORT="$MAIL_SMTP_PORT" THRUST_SMTP_TLS=none THRUST_SMTP_FROM="Thrust <no-reply@thrust.test>"
  export THRUST_PUBLIC_BASE_URL="$THRUST_BASE_URL"
  export THRUST_MAIL_API_URL="http://127.0.0.1:$MAIL_HTTP_PORT"
  echo "[$TARGET/$PASS] mail server (Mailpit) up: smtp $MAIL_SMTP_PORT, api $MAIL_HTTP_PORT"
fi

if [ "$KIND" = "native" ]; then
  # THRUST_SERVER_FEATURES is only ever set by scripts/mutants.sh (control runs with broken builds).
  # The bench and scale passes measure a release build; every other pass uses the debug build.
  if [ "$PASS" = "bench" ] || [ "$PASS" = "scale" ]; then PROFILE_FLAG="--release"; BIN_DIR=release; else PROFILE_FLAG=""; BIN_DIR=debug; fi
  # shellcheck disable=SC2086
  "$ROOT/scripts/rs" cargo build --quiet $PROFILE_FLAG --manifest-path "$ROOT/Cargo.toml" -p thrust-server ${THRUST_SERVER_FEATURES:+--features "$THRUST_SERVER_FEATURES"} \
    >"$LOG.build.log" 2>&1 || { echo "server build failed, see $LOG.build.log" >&2; exit 3; }
  # `default` sets nothing, so the server's own default decides
  [ "$CSRF_SERVER" = default ] || export THRUST_CSRF_ENFORCE="$CSRF_SERVER"
  THRUST_LISTEN="127.0.0.1:$PORT" THRUST_ADAPTERS="$ADAPTERS" THRUST_TEST_ROUTES=1 \
    THRUST_OUTBOX_FASTPATH="$FASTPATH" THRUST_SWEEP_INTERVAL_SECS=0 \
    "$ROOT/target/$BIN_DIR/thrust-server" >"$LOG.server.log" 2>&1 &
  SERVER_PID=$!
else
  # Worker: `wrangler dev` (local workerd). See scripts/worker-dev.sh. The e2e pass has it serve the app's files too.
  if [ "$PASS" = "e2e" ] || [ "$PASS" = "e2e-mail" ]; then export THRUST_ASSETS_DIR="$E2E_ASSETS_DIR"; fi
  THRUST_PORT="$PORT" THRUST_ADAPTERS="$ADAPTERS" THRUST_OUTBOX_FASTPATH="$FASTPATH" \
    THRUST_CSRF_ENFORCE="$CSRF_SERVER" \
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
    # A refused bind (issue #93): name what holds the port, and in which state, so a recurrence explains itself.
    if grep -qE 'AddrInUse|Address already in use' "$LOG.server.log" && command -v ss >/dev/null 2>&1; then
      echo "port $PORT was refused; the sockets on it (ss):" >&2
      ss -tan "( sport = :$PORT )" >&2 || true
      # A port that is listening was taken by something this run started, or by something else: say what this run holds.
      echo "the ports of this run, in the order they were drawn or given: $(tr '\n' ' ' <"$PORTS_DRAWN_FILE")" >&2
      echo "  server $PORT, mock identity provider $THRUST_MOCK_IDP_PORT, postgres ${PG_PORT:-none}," \
        "mail ${MAIL_SMTP_PORT:-none}/${MAIL_HTTP_PORT:-none}, delay proxy ${PROXY_PORT:-none}" >&2
      if command -v docker >/dev/null 2>&1; then
        docker ps --format '  container {{.Names}} publishes {{.Ports}}' >&2 || true
      fi
    fi
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

# The same guard for the CSRF switch: a main pass against a server that does not enforce the token would pass every CSRF
# test that does not look (and fail the ones that do, for the wrong reason), and a `csrfoff` pass against one that does
# enforce it would be a pass that proves nothing about the switch. /_info says what the server was started with.
case "$INFO" in
  *"\"csrf_enforce\":$WANT_CSRF"*) ;;
  *) echo "[$TARGET/$PASS] VACUITY GUARD: csrf_enforce is not $WANT_CSRF (pass '$PASS' needs enforcement $CSRF)" >&2; exit 5 ;;
esac

# The same guard for mail: the `mail` pass against a target with no mail service would fail its tests for the wrong reason (and every other
# pass against one that has one would pass the dark test for none), so the app's own question, `/api/instances/`, must say what the pass needs.
if [ "$PASS" = mail ] || [ "$PASS" = e2e-mail ]; then WANT_SMTP=true; else WANT_SMTP=false; fi
INSTANCES="$(curl -fsS "$THRUST_BASE_URL/api/instances/" 2>/dev/null || true)"
case "$INSTANCES" in
  *"\"is_smtp_configured\":$WANT_SMTP"*) ;;
  *) echo "[$TARGET/$PASS] VACUITY GUARD: is_smtp_configured is not $WANT_SMTP, which pass '$PASS' needs: $INSTANCES" >&2; exit 5 ;;
esac

if [ "$TARGET" = "worker-memory" ]; then
  # state lives inside the isolate; a panic (abort) resets it
  THRUST_STATE_SURVIVES_PANIC=0
else
  THRUST_STATE_SURVIVES_PANIC=1
fi
export THRUST_STATE_SURVIVES_PANIC

# suite_pass BIN FILTERSET [NEXTEST ARGS...]: one pass of the suite, on nextest with the `contract` profile. BIN is the test binary,
# FILTERSET the pass's filterset over it (keep it single-quoted on one line: scripts/check-contract-counts.sh reads it from here and
# lists it, to hold EXPECT_* to the suite), and the rest nextest's own arguments: `--no-capture` runs the tests one at a time and
# prints what they print, which is what the passes that read a test's own output need. Sets TEST_STATUS, and moves the JUnit
# report to `$LOG.junit.xml` (a stale one is removed first, so a run that wrote none is read as none).
suite_pass() {
  sp_bin="$1"
  sp_set="$2"
  shift 2
  sp_junit="${CARGO_TARGET_DIR:-$ROOT/target}/nextest/contract/junit.xml"
  rm -f "$sp_junit" "$LOG.junit.xml"
  "$ROOT/scripts/rs" cargo nextest run --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --profile contract \
    -E "binary($sp_bin) & ($sp_set)" "$@" >"$LOG.test.log" 2>&1
  TEST_STATUS=$?
  if [ -f "$sp_junit" ]; then mv "$sp_junit" "$LOG.junit.xml"; fi
}

# load_pass PREFIX: the load tests whose names start with PREFIX (thrust-load-tests, crates/testing/suites/load), release-built, with
# the `load` profile: one at a time, each printing what it measured. Not a count: a load test passes when every request it made
# succeeded, and no number gates. Sets TEST_STATUS; nextest's 4 (no test ran: the prefix selects nothing) becomes 5, as a pass's does.
load_pass() {
  "$ROOT/scripts/rs" cargo nextest run --manifest-path "$ROOT/Cargo.toml" --cargo-profile release -p thrust-load-tests --profile load \
    -E "test(/^$1/)" >"$LOG.test.log" 2>&1
  TEST_STATUS=$?
  cat "$LOG.test.log"
  if [ "$TEST_STATUS" -eq 4 ]; then
    echo "[$TARGET/$PASS] VACUITY GUARD: no load test ran (nextest exit 4: nothing is named $1)" >&2
    TEST_STATUS=5
  fi
}

case "$PASS" in
  sweeper)
    suite_pass contract 'test(/sweeper_/)' --no-capture
    EXPECT=$EXPECT_SWEEPER
    ;;
  csrfoff)
    suite_pass contract 'test(/csrf_off_/)'
    EXPECT=$EXPECT_CSRFOFF
    ;;
  panic)
    suite_pass contract 'test(/panic_/)' --no-capture
    EXPECT=$EXPECT_PANIC
    ;;
  blast)
    # OBSERVATIONAL (not in contract-all): panics the handler while 16 clients run an I/O-bound
    # request and reports how many OTHER requests failed. Under `wrangler dev` the memory set can
    # lose the whole dev server here, which is itself an observation, so this is run by hand.
    suite_pass contract 'test(/blast_/)' --no-capture
    EXPECT=$EXPECT_BLAST
    ;;
  worker)
    suite_pass worker_only 'test(/worker_room_/)' --no-capture
    EXPECT=$EXPECT_WORKER
    ;;
  bench)
    # The latency load tests (one request at a time, then eight at once; THRUST_LOAD_ITERATIONS creates each, 200 by default)
    # against the target this script just started. Numbers are LOCAL (loopback), see research/0004.
    load_pass load_latency_
    echo "[$TARGET/$PASS] bench exit=$TEST_STATUS"
    exit "$TEST_STATUS"
    ;;
  engine)
    suite_pass engine 'all()' --no-capture
    EXPECT=$EXPECT_ENGINE
    ;;
  oracle)
    # The v1 oracle's scoped gate and corpus comparison against the target just started (scripts/oracle-v1.sh <target> runs this
    # pass, with the label and the controls it was asked for; a cell reaches it directly and gets the target's name and the controls,
    # which cost seconds on a Thrust target). Not a test count: success == the script's verdict, `VERDICT: PASS`. Its logs sit beside
    # this pass's own, so that the cell's log artifact holds them.
    ORACLE_V1_LABEL="${ORACLE_V1_LABEL:-$TARGET}" ORACLE_V1_WHAT="${ORACLE_V1_WHAT-controls}" ORACLE_V1_WORK="${ORACLE_V1_WORK:-$LOG.oracle}" \
      "$ROOT/scripts/oracle-v1.sh" running
    TEST_STATUS=$?
    echo "[$TARGET/$PASS] oracle exit=$TEST_STATUS"
    exit "$TEST_STATUS"
    ;;
  e2e)
    # The first browser journey against the Worker just started (scripts/e2e.sh <target> runs this pass, with the controls it was asked
    # for; a cell reaches it directly and gets them). Not a test count: success == the script's verdict, `VERDICT: PASS`. Its logs
    # and the network's HAR sit beside this pass's own, so that the cell's log artifact holds them.
    E2E_WHAT="${E2E_WHAT-controls}" E2E_OUT="${E2E_OUT:-$LOG.e2e}" "$ROOT/scripts/e2e.sh" running
    TEST_STATUS=$?
    echo "[$TARGET/$PASS] e2e exit=$TEST_STATUS"
    exit "$TEST_STATUS"
    ;;
  e2e-mail)
    # The same, for the journeys that need mail (e2e/journeys-mail/): the Worker above has the mail service, and the journey reads the code
    # it is mailed from THRUST_MAIL_WRANGLER_LOG, which this script exported above and which the browser run inherits.
    E2E_SUITE=mail E2E_WHAT="${E2E_WHAT-controls}" E2E_OUT="${E2E_OUT:-$LOG.e2e}" "$ROOT/scripts/e2e.sh" running
    TEST_STATUS=$?
    echo "[$TARGET/$PASS] e2e-mail exit=$TEST_STATUS"
    exit "$TEST_STATUS"
    ;;
  scale)
    # The scale load test, which reads THRUST_SCALE_TOTAL and THRUST_SCALE_PER_PROJECT (100000 and 1000) itself.
    load_pass load_scale_
    echo "[$TARGET/$PASS] scale exit=$TEST_STATUS"
    exit "$TEST_STATUS"
    ;;
  bindings)
    suite_pass worker_only 'test(/worker_bindings_/)' --no-capture
    EXPECT=$EXPECT_BINDINGS
    ;;
  cron)
    suite_pass worker_only 'test(/worker_cron_/)' --no-capture
    EXPECT=$EXPECT_CRON
    ;;
  mail)
    # The password reset by email through a real mail server (its own test binary, `--test mail`): not part of `main`, which holds that the
    # route is dark on a target with no mail service.
    suite_pass mail 'all()' --success-output immediate --failure-output immediate
    EXPECT=$EXPECT_MAIL
    ;;
  budget)
    # Only the round-trip budget test (issue #39), which is also part of `main`: scripts/mutants.sh runs a mutant of the counter
    # against this and not against the whole main pass. Not in contract-all.
    suite_pass contract 'test(/list_endpoints_stay_within_five_db_round_trips/)' --no-capture
    EXPECT=$EXPECT_BUDGET
    ;;
  *)
    suite_pass contract 'not test(/sweeper_|panic_|blast_|csrf_off_/)'
    EXPECT=$EXPECT_MAIN
    ;;
esac

grep -E "PANIC-OBSERVATION|HIBERNATION-OBSERVATION" "$LOG.test.log" || true

grep -E '^ +(PASS|FAIL|SIG[A-Z]+|TIMEOUT|LEAK|SLOW) |Summary \[|panicked' "$LOG.test.log"
# The count, from nextest's JUnit report: `<testsuites ... tests="N" failures="F" errors="E" ...>` holds the tests that ran (the
# ones the filterset left out are not in it) and how many of them did not pass. No report (a build that failed) is no count.
junit_attr() { sed -n "s/.*<testsuites[^>]* $2=\"\([0-9][0-9]*\)\".*/\1/p" "$1" 2>/dev/null | head -n 1; }
RAN="$(junit_attr "$LOG.junit.xml" tests)"
FAILURES="$(junit_attr "$LOG.junit.xml" failures)"
ERRORS="$(junit_attr "$LOG.junit.xml" errors)"
PASSED=""
if [ -n "$RAN" ] && [ -n "$FAILURES" ] && [ -n "$ERRORS" ]; then PASSED=$((RAN - FAILURES - ERRORS)); fi
echo "[$TARGET/$PASS] nextest exit=$TEST_STATUS passed=${PASSED:-?} expected=$EXPECT"
if [ "$TEST_STATUS" -eq 4 ]; then
  echo "[$TARGET/$PASS] VACUITY GUARD: no test ran (nextest exit 4: the filterset selects nothing)" >&2
  exit 5
fi
if [ "$TEST_STATUS" -eq 0 ] && [ "${PASSED:-0}" -ne "$EXPECT" ]; then
  echo "[$TARGET/$PASS] VACUITY GUARD: exit 0 but passed count != expected" >&2
  exit 5
fi
exit "$TEST_STATUS"
