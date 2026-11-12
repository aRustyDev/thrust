#!/bin/sh
# The browser journeys (issue #44 built the first, issue #45 makes them a suite): the stock Plane app, built from web/, in a real
# Chromium, against a Worker that serves it. Each journey is one spec in e2e/journeys/. The first signs a user up, onboards them, makes
# a workspace, a project and a work item, and lists it, and fails on any 404 or 5xx in the network log (the specs and e2e/support/
# say what else would make a journey pass without proving anything).
#
#   scripts/e2e.sh <target> [controls]
#     target    worker-cf | worker-memory
#                         a Worker started the way scripts/contract.sh starts it (its `e2e` pass: the app's files served by the
#                         Worker, CSRF enforcement at the Worker's own default, which is off) and stopped after. The native server
#                         does not serve the app yet (ADR-0011, round 29), so there is no native target
#               running   a Worker that is already up and serves the app: THRUST_BASE_URL and THRUST_ADMIN_TOKEN. The journey makes
#                         a user, a workspace and a project in it, and leaves them there
#     controls  also the controls, each a run of the journey that MUST fail, and fail by saying why (a pass proves nothing until its
#               detectors have been shown to fire):
#                 stickies-404  one route answers 404 in the browser; the journey must name that 404
#                 empty-list    the browser empties the work-item list (S47); the journey must fail at "see it in the list"
#                 skip          the journey skips itself, which Playwright counts as a pass; the count guard must refuse it
#                 wrong target  the journey told it is on the other target (Plane, for a Thrust) must fail at the TARGET GUARD
#
# The count guard: every run's own JSON report must say that every journey in e2e/journeys/ passed, and none was skipped or flaky
# (e2e/support/check-report.mjs). The number of journeys is the number of files, so a filter or a skip cannot select fewer and pass.
#
# The app must be built first (the result is web/apps/web/build/client): this script builds it when it is not, with the pnpm and Node
# that web/ pins. In CI the cell builds it in steps of its own (.github/workflows/contract-cell.yml).
#
# Logs, each run's report, the HAR of the network and the browser's artifacts for a failure: $E2E_OUT, default
# target/contract-logs/<target>-e2e.e2e/. Last line `VERDICT: PASS` and exit 0, or `VERDICT: FAIL` and exit 1. Exit 2: a usage or
# set-up problem.
#
# The exit status of each run is read from the run itself (a redirect, never a pipe): a pipe would report the status of its last
# command. POSIX sh: CI runs it under dash.
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
# which target the journeys are told they are on: `thrust` here, and `plane` for the pinned oracle (issue #45)
WANT="${E2E_TARGET:-thrust}"
case "$WANT" in thrust) OTHER=plane ;; plane) OTHER=thrust ;; *) echo "e2e: E2E_TARGET is '$WANT': it is thrust or plane" >&2; exit 2 ;; esac
OUT="${E2E_OUT:-$ROOT/target/contract-logs/e2e.e2e}"
rm -rf "$OUT"
mkdir -p "$OUT"
export E2E_OUT="$OUT"

# one journey per file: this is the number the count guard holds the report to
count_journeys() {
  set -- "$ROOT"/e2e/journeys/*.spec.ts
  if [ -e "$1" ]; then echo "$#"; else echo 0; fi
}
JOURNEYS="$(count_journeys)"
[ "$JOURNEYS" -ge 1 ] || { echo "e2e: there are no journeys in $ROOT/e2e/journeys/" >&2; exit 2; }

# the journey's own dependencies, at the lockfile; and the browser they drive (a no-op when it is there)
pnpm --dir "$ROOT/e2e" install --frozen-lockfile --ignore-scripts > "$OUT/install.log" 2>&1 ||
  { echo "e2e: the install of e2e/ failed; the end of $OUT/install.log:" >&2; tail -8 "$OUT/install.log" >&2; exit 2; }
pnpm --dir "$ROOT/e2e" exec playwright install chromium > "$OUT/browser.log" 2>&1 ||
  { echo "e2e: could not install the browser; the end of $OUT/browser.log:" >&2; tail -8 "$OUT/browser.log" >&2; exit 2; }

# one run of the journeys: `journey <label> [E2E_CONTROL] [E2E_TARGET]`; its status is the run's, its output is in $OUT/<label>.log and
# its own report in $OUT/<label>.json
journey() {
  E2E_JSON="$OUT/$1.json" E2E_CONTROL="${2:-}" E2E_TARGET="${3:-$WANT}" pnpm --dir "$ROOT/e2e" exec playwright test > "$OUT/$1.log" 2>&1
}

# the count guard on one run's report (it says why on stderr)
check_report() {
  node "$ROOT/e2e/support/check-report.mjs" "$OUT/$1.json" "$JOURNEYS"
}

VERDICT=PASS

# a run that must fail, and fail by saying why: `must_fail <label> <what it was told> <words the failure must hold> [control] [target]`
must_fail() {
  journey "$1" "${4:-}" "${5:-}"
  if [ "$?" -eq 0 ]; then
    echo "e2e: CONTROL FAILED: the journey passed $2, so it cannot see that" >&2
    VERDICT=FAIL
  elif ! grep -q "$3" "$OUT/$1.log"; then
    echo "e2e: CONTROL FAILED: the journey failed $2, but not by saying '$3'; the end of its log:" >&2
    tail -30 "$OUT/$1.log" >&2
    VERDICT=FAIL
  else
    echo "e2e: control ok: the journey fails $2, saying '$3' (log: $OUT/$1.log)"
  fi
}

journey main
STATUS=$?
echo "e2e: the journey run exited $STATUS (log: $OUT/main.log)"
if [ "$STATUS" -ne 0 ]; then
  VERDICT=FAIL
  tail -40 "$OUT/main.log"
elif ! check_report main; then
  # exit 0 and still not every journey passed: skipped, or fewer than there are
  VERDICT=FAIL
fi

if [ "$WHAT" = controls ]; then
  must_fail control-stickies-404 "with the stickies list answering 404" "an unexpected 404: GET /api/workspaces/.*/stickies/ -> 404" stickies-404
  must_fail control-empty-list "with the work-item list emptied" "see it in the list:" empty-list
  must_fail control-wrong-target "told it is on $OTHER" "TARGET GUARD ($OTHER)" "" "$OTHER"

  # the skip control is the other way round: Playwright exits 0, and it is the count guard that must refuse the run
  journey control-skip skip
  if [ "$?" -ne 0 ]; then
    echo "e2e: CONTROL FAILED: the skipped journey did not exit 0, so the control does not test the count guard; the end of its log:" >&2
    tail -20 "$OUT/control-skip.log" >&2
    VERDICT=FAIL
  elif check_report control-skip > "$OUT/control-skip.report" 2>&1; then
    echo "e2e: CONTROL FAILED: the count guard passed a run in which the journey was skipped" >&2
    VERDICT=FAIL
  else
    echo "e2e: control ok: the count guard refuses a skipped journey ($(tail -1 "$OUT/control-skip.report"))"
  fi
fi

echo "VERDICT: $VERDICT"
[ "$VERDICT" = PASS ]
