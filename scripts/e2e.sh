#!/bin/sh
# The browser journeys (issue #44 built the first, issue #45 makes them a suite): the stock Plane app in a real Chromium, against a
# Worker that serves the app built from web/, and against the pinned Plane, the UI oracle, that serves the app built from the pin. Each
# journey is one spec in e2e/journeys/. The first signs a user up, onboards them, makes a workspace, a project and a work item, and
# lists it, and fails on any 404 or 5xx in the network log (the specs and e2e/support/ say what else would make a journey pass without
# proving anything). Green on Plane and red on Thrust is a parity defect; red on Plane means the journey is wrong.
#
#   scripts/e2e.sh <target> [controls]
#     target    worker-cf | worker-memory
#                         a Worker started the way scripts/contract.sh starts it (its `e2e` pass: the app's files served by the
#                         Worker, CSRF enforcement at the Worker's own default, which is off) and stopped after. The native server
#                         does not serve the app yet (ADR-0011, round 29), so there is no native target
#               running   a Worker that is already up and serves the app: THRUST_BASE_URL and THRUST_ADMIN_TOKEN. The journey makes
#                         a user, a workspace and a project in it, and leaves them there
#               plane-oracle
#                         the pinned Plane (tools/oracle-v1, issue #45). LOCAL ONLY: the oracle builds from the fork checkout, which is not
#                         pushed (Q-0122.4). ORACLE_DIR is the oracle/ directory of that checkout, as for tools/compat-record/fresh-oracle.sh.
#                         It builds the pin's stock app (tools/e2e-plane/build-pin-bundle.sh: web/ with the Q-R2 ledger reverted to the pin),
#                         brings the stack up FRESH beside it on one origin (http://localhost:18088; tools/e2e-plane/compose.ui.yml), does
#                         the instance-admin setup the stock app needs, runs the journeys and takes the stack down. `down -v` drops the
#                         stack's volumes, which includes the seeded snapshot that scripts/oracle-v1.sh plane needs: that must be
#                         re-seeded after this. E2E_KEEP=1 leaves the stack up
#     controls  also the controls: runs of the journey that MUST fail, and fail by saying why (a pass proves nothing until its
#               detectors have been shown to fire), and one that MUST PASS (late-profile, below):
#                 stickies-404  one route answers 404 in the browser; the journey must name that 404
#                 empty-list    the browser empties the work-item list (S47); the journey must fail at "see it in the list"
#                 sso-on        the browser turns two sign-in providers on (`/api/instances/`); the sign-in journey must fail at
#                               "a sign-in method other than the password"
#                 skip          the journey skips itself, which Playwright counts as a pass; the count guard must refuse it
#                 wrong target  the journey told it is on the other target must fail at the TARGET GUARD (a Thrust told it is on Plane,
#                               and the pinned Plane told it is a Thrust)
#                 slow-page     the one that must PASS: the browser is slowed to a CI runner's pace (CPU 6x, E2E_CPU_RATE) and the profile
#                               answer that the home page asks for as it mounts is held for 2 s. A journey that is right on a fast
#                               machine only, such as one that answers the welcome tour the moment it shows, fails it (G-31)
#
#   E2E_RECORD_HAR=1   after a passing run, scrub the network log of each journey (tools/e2e-har/scrub.py) into e2e/har/<journey>.<target>.har,
#                      with the bodies of the /api/ and /auth/ calls: the input of spike 0009's replay harness. Commit what it writes
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

TARGET="${1:?usage: e2e.sh <worker-cf|worker-memory|running|plane-oracle> [controls]}"
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
  running | plane-oracle) ;;
  *) echo "e2e: unknown target '$TARGET'" >&2; exit 2 ;;
esac

if [ "$TARGET" = plane-oracle ]; then
  # the pinned Plane is the one target the journeys are told is `plane`; its browser origin is what compose.ui.yml tells the API
  # (WEB_URL and the others): `localhost`, not 127.0.0.1, because the cookies are host-scoped and the 302 after a sign-in goes to that name
  case "${E2E_TARGET:-plane}" in plane) ;; *) echo "e2e: plane-oracle is the plane target, and E2E_TARGET is '$E2E_TARGET'" >&2; exit 2 ;; esac
  E2E_TARGET=plane
  THRUST_BASE_URL="http://localhost:18088"
  # the pinned Plane has no admin token; the target guard sends this one in a header that Plane ignores
  THRUST_ADMIN_TOKEN="${THRUST_ADMIN_TOKEN:-none}"
  export E2E_TARGET
else
  : "${THRUST_BASE_URL:?e2e: running needs THRUST_BASE_URL}"
  : "${THRUST_ADMIN_TOKEN:?e2e: running needs THRUST_ADMIN_TOKEN}"
fi
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

# the HAR keeps the bodies of the API's calls only when it is to be committed (e2e/support/context.ts)
if [ "${E2E_RECORD_HAR:-}" = 1 ]; then E2E_HAR_BODIES=1; else E2E_HAR_BODIES=""; fi
export E2E_HAR_BODIES

if [ "$TARGET" = plane-oracle ]; then
  : "${ORACLE_DIR:?e2e: plane-oracle needs ORACLE_DIR, the oracle/ directory of the Plane fork checkout on the pin branch (as for tools/compat-record/fresh-oracle.sh)}"
  [ -s "$ORACLE_DIR/compose.oracle.yml" ] ||
    { echo "e2e: $ORACLE_DIR/compose.oracle.yml is missing: ORACLE_DIR is the oracle/ directory of the fork checkout" >&2; exit 2; }
  ORACLE_DIR="$(cd "$ORACLE_DIR" && pwd)"
  # the bundle is the pin's app; the checkout is only read, by `git show` (the script checks that it holds the pin)
  BUNDLE="$("$ROOT/tools/e2e-plane/build-pin-bundle.sh" "$(cd "$ORACLE_DIR/.." && pwd)")" ||
    { echo "e2e: could not build the pin's app (the messages are above)" >&2; exit 2; }
  export E2E_PLANE_DIR="$ROOT/tools/e2e-plane" E2E_PLANE_BUNDLE="$BUNDLE" ORACLE_DIR

  oracle_compose() {
    docker compose -f "$ORACLE_DIR/compose.oracle.yml" -f "$ROOT/tools/compat-record/oracle-assets.yml" -f "$E2E_PLANE_DIR/compose.ui.yml" \
      --env-file "$ORACLE_DIR/.env.oracle" "$@"
  }
  # the stack comes down however this ends (a trap keeps the status the script was ending with), unless it was asked to stay
  oracle_down() {
    if [ "${E2E_KEEP:-}" != 1 ]; then
      oracle_compose down -v --remove-orphans > "$OUT/oracle-down.log" 2>&1
    else
      echo "e2e: E2E_KEEP=1: the oracle stack is left up (docker compose -p plane-oracle down -v to remove it)"
    fi
  }
  trap oracle_down EXIT

  echo "e2e: bringing the pinned Plane up fresh (log: $OUT/oracle-up.log)"
  ORACLE_COMPOSE_EXTRA="$E2E_PLANE_DIR/compose.ui.yml" "$ROOT/tools/compat-record/fresh-oracle.sh" > "$OUT/oracle-up.log" 2>&1 ||
    { echo "e2e: the oracle stack did not come up; the end of $OUT/oracle-up.log:" >&2; tail -15 "$OUT/oracle-up.log" >&2; exit 2; }
  # a fresh pin sends every visitor to the instance-admin setup until it is done (the recorder does the same before it records)
  uv run --script "$ROOT/tools/compat-record/record.py" --setup-instance --base-url http://127.0.0.1:18080 > "$OUT/instance-setup.log" 2>&1 ||
    { echo "e2e: the instance-admin setup failed; the end of $OUT/instance-setup.log:" >&2; tail -8 "$OUT/instance-setup.log" >&2; exit 2; }
fi

# one run of the journeys: `journey <label> [E2E_CONTROL] [E2E_TARGET]`; its status is the run's, its output is in $OUT/<label>.log, its
# own report in $OUT/<label>.json and its network log in $OUT/har-<label>/ (one directory per run, so that a control cannot overwrite it)
journey() {
  mkdir -p "$OUT/har-$1"
  E2E_JSON="$OUT/$1.json" E2E_HAR_DIR="$OUT/har-$1" E2E_CONTROL="${2:-}" E2E_TARGET="${3:-$WANT}" \
    pnpm --dir "$ROOT/e2e" exec playwright test > "$OUT/$1.log" 2>&1
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

# a run that must PASS although it is told something that makes a journey which is not robust fail: `must_pass <label> <what it was told> <control>`.
# It is held to what the main run is held to: the exit status, and then the count guard (the control itself says CONTROL INERT if it did nothing)
must_pass() {
  journey "$1" "$3"
  if [ "$?" -ne 0 ]; then
    echo "e2e: CONTROL FAILED: the journey failed $2, and it must not; the end of its log:" >&2
    tail -30 "$OUT/$1.log" >&2
    VERDICT=FAIL
  elif ! check_report "$1"; then
    VERDICT=FAIL
  else
    echo "e2e: control ok: the journey passes $2 (log: $OUT/$1.log)"
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

# the committed HAR: only of a run that passed, one per journey, scrubbed (tools/e2e-har/scrub.py refuses one that has nothing to scrub)
if [ "$VERDICT" = PASS ] && [ "${E2E_RECORD_HAR:-}" = 1 ]; then
  HARS=0
  for har in "$OUT"/har-main/*.har; do
    [ -e "$har" ] || continue
    name="$(basename "$har" .har)"
    if uv run --script "$ROOT/tools/e2e-har/scrub.py" --out "$ROOT/e2e/har/$name.$WANT.har" "$har"; then
      HARS=$((HARS + 1))
    else
      echo "e2e: the scrub of $har failed" >&2
      VERDICT=FAIL
    fi
  done
  if [ "$HARS" -ne "$JOURNEYS" ]; then
    echo "e2e: $HARS HAR(s) written for $JOURNEYS journey(s): there must be one each" >&2
    VERDICT=FAIL
  fi
fi

if [ "$WHAT" = controls ]; then
  must_fail control-stickies-404 "with the stickies list answering 404" "an unexpected 404: GET /api/workspaces/.*/stickies/ -> 404" stickies-404
  must_fail control-empty-list "with the work-item list emptied" "see it in the list:" empty-list
  must_fail control-sso-on "with two sign-in providers turned on in the browser" "a sign-in method other than the password" sso-on
  must_fail control-wrong-target "told it is on $OTHER" "TARGET GUARD ($OTHER)" "" "$OTHER"
  must_pass control-slow-page "with the browser slowed to a CI runner's pace and the profile answer of the home page late" slow-page

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
