#!/bin/sh
# STAGE 2 ONLY — runs the ONE contract suite against a DEPLOYED Worker (see research/0004 §10).
# Counterpart of `contract.sh worker-cf <pass>`, except the target is already running on Cloudflare.
#
#   THRUST_BASE_URL=https://<worker>.<sub>.workers.dev \
#   THRUST_ADMIN_TOKEN_FILE=<0600 file holding the Worker's THRUST_ADMIN_TOKEN> \
#     scripts/stage2-contract.sh [main|worker|bindings|cron|sweeper|panic]
#
# main      31 tests: the portable contract minus the 6 `oidc_` tests (they need the mock IdP on 127.0.0.1, which a
#           deployed Worker cannot reach) (the deployed Worker must have THRUST_CREATE_RATE_LIMIT=30, and a cheap
#           argon2id cost, THRUST_ARGON2_M_KIB=64 THRUST_ARGON2_T=1 THRUST_ARGON2_P=1, or every fixture seeds six
#           passwords at the real cost)
# worker     1 test : Durable Object WebSocket hibernation + SQLite
# bindings   6 tests: one real operation per binding (rate limit, secrets store, email, service, workflow)
# cron       1 test : needs the Worker deployed with THRUST_OUTBOX_FASTPATH=off
# sweeper    2 tests: needs THRUST_OUTBOX_FASTPATH=off
# panic      1 test : alone, against a Worker nobody else is using (a panic resets the isolate)
# csrfoff    6 tests: needs the Worker deployed with THRUST_CSRF_ENFORCE=off (Q-0050.8, the production default while the
#            stock frontend is in use). Every other pass needs it ON, which wrangler.stage2.toml sets; /_info is checked
#
# The admin token is read from the file into THIS process's environment only: never on argv, never
# printed. Exit status is cargo test's own (no pipe, no trailing echo); the passed count is checked
# against the expected number so a filter selecting zero tests cannot report green.
set -u
PASS="${1:-main}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BASE="${THRUST_BASE_URL:?set THRUST_BASE_URL to the deployed Worker}"
TOKEN_FILE="${THRUST_ADMIN_TOKEN_FILE:?set THRUST_ADMIN_TOKEN_FILE to the 0600 token file}"
[ -r "$TOKEN_FILE" ] || { echo "token file not readable" >&2; exit 2; }
THRUST_ADMIN_TOKEN="$(cat "$TOKEN_FILE")"
[ "${#THRUST_ADMIN_TOKEN}" -ge 32 ] || { echo "token file too short" >&2; exit 2; }
export THRUST_ADMIN_TOKEN
export THRUST_BASE_URL="$BASE"
export THRUST_EXPECT_CREATE_LIMIT="${THRUST_EXPECT_CREATE_LIMIT:-30}"
export THRUST_STATE_SURVIVES_PANIC=1
LOGDIR="${THRUST_LOGDIR:-$ROOT/target/stage2-logs}"
mkdir -p "$LOGDIR"
LOG="$LOGDIR/worker-cf-$PASS"

# CSRF enforcement (Q-0050.8): every pass needs a Worker that enforces the token, except `csrfoff`, which needs one that does
# not. The deployed Worker says which it is; a pass against the other kind proves nothing, so it is refused.
case "$PASS" in csrfoff) WANT_CSRF=false ;; *) WANT_CSRF=true ;; esac

# Vacuity guard: the deployed Worker must say it is the Cloudflare adapter set, on the Worker target.
INFO="$(curl -fsS --max-time 30 -H "x-admin-token: $THRUST_ADMIN_TOKEN" "$BASE/_info" 2>/dev/null || true)"
echo "[stage2/$PASS] /_info: $INFO"
case "$INFO" in *'"target":"worker"'*) ;; *) echo "[stage2/$PASS] VACUITY GUARD: not a Worker target" >&2; exit 5 ;; esac
case "$INFO" in *'"adapters":"cloudflare"'*) ;; *) echo "[stage2/$PASS] VACUITY GUARD: adapters not cloudflare" >&2; exit 5 ;; esac
case "$INFO" in *"\"csrf_enforce\":$WANT_CSRF"*) ;; *) echo "[stage2/$PASS] VACUITY GUARD: csrf_enforce is not $WANT_CSRF (pass '$PASS')" >&2; exit 5 ;; esac

RS="$ROOT/scripts/rs"
M="$ROOT/Cargo.toml"
case "$PASS" in
  main)     EXPECT=31; "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test contract -- --skip sweeper_ --skip panic_ --skip blast_ --skip oidc_ --skip csrf_off_ >"$LOG.test.log" 2>&1 ;;
  # the csrf_off_ tests that need nothing else the deployed Worker lacks (all six: none of them is an `oidc_` test)
  csrfoff)  EXPECT=6;  "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test contract -- csrf_off_ >"$LOG.test.log" 2>&1 ;;
  worker)   EXPECT=1;  "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test worker_only -- worker_room_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1 ;;
  # Email is skipped on purpose: a real send needs a sender domain owned by the account and a verified
  # destination; the deployed binding rejects `example.com` ("domain is not owned by the same account").
  # It is reported as NOT exercised, never as passed.
  bindings) EXPECT=5;  "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test worker_only -- worker_bindings_ --skip email_send --test-threads=1 --nocapture >"$LOG.test.log" 2>&1 ;;
  cron)     EXPECT=1;  "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test worker_only -- worker_cron_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1 ;;
  # crontrigger: the REAL every-minute trigger (the `cron` pass uses wrangler dev's /__scheduled, which a deployed Worker lacks)
  crontrigger) EXPECT=1; "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test worker_only -- stage2_cron_real_trigger_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1 ;;
  sweeper)  EXPECT=2;  "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test contract -- sweeper_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1 ;;
  panic)    EXPECT=1;  "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test contract -- panic_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1 ;;
  # blast is OBSERVATIONAL: how many OTHER in-flight requests fail around one panic (a panic aborts the isolate)
  blast)    EXPECT=1;  "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test contract -- blast_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1 ;;
  ratelimit) EXPECT=1; "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test contract -- create_is_rate_limited_per_user --test-threads=1 --nocapture >"$LOG.test.log" 2>&1 ;;
  *) echo "unknown pass $PASS" >&2; exit 2 ;;
esac
TEST_STATUS=$?

grep -E "PANIC-OBSERVATION|HIBERNATION-OBSERVATION|RATELIMIT-OBSERVATION|CRON-OBSERVATION" "$LOG.test.log" || true
grep -E "^test |^test result|panicked|FAILED" "$LOG.test.log"
PASSED="$(sed -n 's/^test result: [a-zA-Z]*\. \([0-9]*\) passed.*/\1/p' "$LOG.test.log" | tail -1)"
echo "[stage2/$PASS] cargo test exit=$TEST_STATUS passed=${PASSED:-?} expected=$EXPECT"
if [ "$TEST_STATUS" -eq 0 ] && [ "${PASSED:-0}" -ne "$EXPECT" ]; then
  echo "[stage2/$PASS] VACUITY GUARD: exit 0 but passed count != expected" >&2
  exit 5
fi
exit "$TEST_STATUS"
