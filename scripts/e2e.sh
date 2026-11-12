#!/bin/sh
# The first browser journey (issue #44, becoming the first of issue #45): the stock Plane app, built from web/, in a real Chromium,
# against a Worker that serves it. It signs a user up, onboards them, makes a workspace, a project and a work item, and lists it, and
# fails on any 404 or 5xx in the network log (e2e/journey.spec.ts says what else would make it pass without proving anything).
#
#   scripts/e2e.sh <target> [controls]
#     target    worker-cf | worker-memory
#                         a Worker started the way scripts/contract.sh starts it (its `e2e` pass: the app's files served by the
#                         Worker, CSRF enforcement at the Worker's own default, which is off) and stopped after. The native server
#                         does not serve the app yet (ADR-0011, round 29), so there is no native target
#               running   a Worker that is already up and serves the app: THRUST_BASE_URL and THRUST_ADMIN_TOKEN. The journey makes
#                         a user, a workspace and a project in it, and leaves them there
#     controls  also the control: the same journey with one route (the stickies list) answering 404 in the browser, which must FAIL,
#               and fail by saying so. A detector that cannot see a 404 would otherwise pass every run
#
# The app must be built first (the result is web/apps/web/build/client): this script builds it when it is not, with the pnpm and Node
# that web/ pins. In CI the cell builds it in steps of its own (.github/workflows/contract-cell.yml).
#
# Logs, the HAR of the network and the browser's artifacts for a failure: $E2E_OUT, default target/contract-logs/<target>-e2e.e2e/.
# Last line `VERDICT: PASS` and exit 0, or `VERDICT: FAIL` and exit 1. Exit 2: a usage or set-up problem.
#
# The exit status of each run is read from the run itself (a redirect, never a pipe): a pipe would report the status of its last
# command.
set -u

TARGET="${1:?usage: e2e.sh <worker-cf|worker-memory|running> [controls]}"
WHAT="${2:-}"
case "$WHAT" in "" | controls) ;; *) echo "e2e: the second argument is 'controls' or nothing, not '$WHAT'" >&2; exit 2 ;; esac
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="${E2E_ASSETS_DIR:-$ROOT/web/apps/web/build/client}"

case "$TARGET" in
  worker-cf | worker-memory)
    if [ ! -s "$APP/index.html" ]; then
      echo "e2e: the app is not built ($APP/index.html is missing); building it"
      pnpm --dir "$ROOT/web" install --frozen-lockfile --ignore-scripts || { echo "e2e: the install of web/ failed" >&2; exit 2; }
      pnpm --dir "$ROOT/web" exec turbo run build --filter=web || { echo "e2e: the build of the app failed" >&2; exit 2; }
      [ -s "$APP/index.html" ] || { echo "e2e: the build made no $APP/index.html" >&2; exit 2; }
    fi
    # start the target as the contract suite does and come back here, with it up, as `running`: contract.sh's `e2e` pass runs this script
    export E2E_ASSETS_DIR="$APP" E2E_WHAT="$WHAT"
    exec "$ROOT/scripts/contract.sh" "$TARGET" e2e
    ;;
  running) ;;
  *) echo "e2e: unknown target '$TARGET'" >&2; exit 2 ;;
esac

: "${THRUST_BASE_URL:?e2e: running needs THRUST_BASE_URL}"
: "${THRUST_ADMIN_TOKEN:?e2e: running needs THRUST_ADMIN_TOKEN}"
export THRUST_BASE_URL THRUST_ADMIN_TOKEN
WHAT="${E2E_WHAT:-$WHAT}"
OUT="${E2E_OUT:-$ROOT/target/contract-logs/e2e.e2e}"
rm -rf "$OUT"
mkdir -p "$OUT"
export E2E_OUT="$OUT"

# the journey's own dependencies, at the lockfile; and the browser they drive (a no-op when it is there)
pnpm --dir "$ROOT/e2e" install --frozen-lockfile --ignore-scripts > "$OUT/install.log" 2>&1 ||
  { echo "e2e: the install of e2e/ failed; the end of $OUT/install.log:" >&2; tail -8 "$OUT/install.log" >&2; exit 2; }
pnpm --dir "$ROOT/e2e" exec playwright install chromium > "$OUT/browser.log" 2>&1 ||
  { echo "e2e: could not install the browser; the end of $OUT/browser.log:" >&2; tail -8 "$OUT/browser.log" >&2; exit 2; }

# one run of the journey: `journey <label> [E2E_CONTROL]`; its status is the run's, and its output is in $OUT/<label>.log
journey() {
  E2E_CONTROL="${2:-}" pnpm --dir "$ROOT/e2e" exec playwright test > "$OUT/$1.log" 2>&1
}

VERDICT=PASS

journey main
STATUS=$?
echo "e2e: the journey exited $STATUS (log: $OUT/main.log)"
if [ "$STATUS" -ne 0 ]; then
  VERDICT=FAIL
  tail -40 "$OUT/main.log"
fi

if [ "$WHAT" = controls ]; then
  # the stickies list answers 404 in the browser: the journey must fail, and say it saw that 404
  journey control-stickies-404 stickies-404
  CSTATUS=$?
  if [ "$CSTATUS" -eq 0 ]; then
    echo "e2e: CONTROL FAILED: the journey passed with the stickies list answering 404, so it cannot see a 404" >&2
    VERDICT=FAIL
  elif ! grep -q "an unexpected 404: GET /api/workspaces/.*/stickies/ -> 404" "$OUT/control-stickies-404.log"; then
    echo "e2e: CONTROL FAILED: the journey failed with the stickies list answering 404, but not by naming that 404; the end of its log:" >&2
    tail -30 "$OUT/control-stickies-404.log" >&2
    VERDICT=FAIL
  else
    echo "e2e: control ok: the journey fails, naming the 404 (log: $OUT/control-stickies-404.log)"
  fi
fi

echo "VERDICT: $VERDICT"
[ "$VERDICT" = PASS ]
