#!/bin/sh
# The v1 oracle's scoped gate and corpus comparison against one target (issue #58, ADR-0122).
#
#   scripts/oracle-v1.sh <target> [controls]
#     target    native-memory | native-pg | worker-memory | worker-cf
#                         a Thrust target, started the way scripts/contract.sh starts it (its `oracle` pass) and stopped after
#               running   a Thrust that is already up: THRUST_BASE_URL and THRUST_ADMIN_TOKEN, started with test routes and with
#                         THRUST_V1_RATE_LIMIT high enough for a fuzzer (the default is Plane's 60/minute). DESTRUCTIVE: its
#                         database is emptied before every step, so it must be one that nobody keeps anything in
#               plane     the pinned Plane, with its stack up (tools/oracle-v1/compose.oracle.yml) and the snapshot of its seeded
#                         database taken (tools/oracle-v1/scripts/db-state.sh snapshot). The same steps, for the numbers they are
#                         measured against: the gate with --target plane, the corpus recorded by the pin itself
#     controls  also the must-fail controls (run_controls.py): C0 passes and C1-C3 fail on GET .../states/; C4 (a State without its
#               `id`, which the schema cannot see) passes the gate and must FAIL the corpus comparison
#
# Steps, each from the oracle's seed (a Thrust target is emptied and re-seeded by scripts/oracle-v1-reset.sh, Plane is restored
# from its snapshot, and a restore that does not give the seed's row counts fails):
#   1. gate      run_conformance.py --scope m1             Schemathesis over the scope's 32 operations, the register restricted to them
#   2. corpus    record_corpus.py --only <resources>, then compare_corpus.py --scope m1: every exchange of the committed corpus
#                that falls on the scope, against the one recorded from the target; a run that compares fewer than the floor fails
#   3. self-test compare_corpus.py --self-test: the comparison passes on the corpus and fails on each injected defect
#   4. controls  (only with `controls`)
# All four are run whatever an earlier one did, so that one run says everything that is wrong; the verdict is the AND.
#
# Results (committed): tools/oracle-v1/results/m1/<label>.{gate,corpus,controls}.json, with <label> the target.
# Last line `VERDICT: PASS` and exit 0, or `VERDICT: FAIL` and exit 1. Exit 2: a usage or set-up problem.
#
# Exit codes of the steps are read from the step itself (a redirect, never a pipe): a pipe would report the status of its last
# command. Secrets: the admin token stays in this process's environment.
set -u

TARGET="${1:?usage: oracle-v1.sh <native-memory|native-pg|worker-memory|worker-cf|running|plane> [controls]}"
WHAT="${2:-}"
case "$WHAT" in "" | controls) ;; *) echo "oracle-v1: the second argument is 'controls' or nothing, not '$WHAT'" >&2; exit 2 ;; esac
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ORACLE="$ROOT/tools/oracle-v1"
H="$ORACLE/harness"
SCOPE=m1

case "$TARGET" in
  native-memory | native-pg | worker-memory | worker-cf)
    # Start the target as the contract suite does (Postgres in Docker for the pg ones, `wrangler dev` for the Workers) and come
    # back here, with the target up, as `running`: contract.sh's `oracle` pass runs this script.
    export ORACLE_V1_LABEL="$TARGET" ORACLE_V1_WHAT="$WHAT"
    exec "$ROOT/scripts/contract.sh" "$TARGET" oracle
    ;;
  running) KIND=thrust; LABEL="${ORACLE_V1_LABEL:-running}"; WHAT="${ORACLE_V1_WHAT:-$WHAT}" ;;
  plane) KIND=plane; LABEL=plane ;;
  *) echo "oracle-v1: unknown target '$TARGET'" >&2; exit 2 ;;
esac

RESULTS="$ORACLE/results/m1"
WORK="$ORACLE/.work/m1-$LABEL"
mkdir -p "$RESULTS" "$WORK"
rm -rf "$WORK/corpus" "$WORK/corpus-raw" "$WORK/conformance"

if [ "$KIND" = thrust ]; then
  URL="${THRUST_BASE_URL:?oracle-v1: running needs THRUST_BASE_URL}"
  : "${THRUST_ADMIN_TOKEN:?oracle-v1: running needs THRUST_ADMIN_TOKEN}"
  export THRUST_BASE_URL THRUST_ADMIN_TOKEN
  RESET="$ROOT/scripts/oracle-v1-reset.sh"
  START_CORPUS_WORKER=""
else
  URL="${ORACLE_PLANE_URL:-http://127.0.0.1:18080}"
  RESET="$ORACLE/scripts/db-state.sh"
  # Plane soft-deletes what a deleted project holds in a Celery task, and the corpus was recorded with that worker running; the gate
  # runs with it stopped, so that no asynchronous task changes the database under it (db-state.sh restore leaves it stopped)
  START_CORPUS_WORKER=1
fi

# Back to the oracle's seed. A Thrust target ignores the argument; the Plane's script wants it.
reset() {
  "$RESET" restore > "$WORK/reset.log" 2>&1 || {
    echo "oracle-v1: could not reset the target to the oracle's seed; the end of $WORK/reset.log:" >&2
    tail -8 "$WORK/reset.log" >&2
    return 1
  }
}

RESOURCES="$(uv run --quiet --with pyyaml python3 -I -c "
import sys, yaml
print(','.join(yaml.safe_load(open('$ORACLE/exceptions.yaml'))['scopes']['$SCOPE']['corpus']['resources']))
")" || { echo "oracle-v1: could not read the scope's resources from the register" >&2; exit 2; }

GATE=1 CORPUS=1 SELF=1 CONTROLS=0

echo "== [$LABEL] gate (scope $SCOPE)"
if reset; then
  uv run --script "$H/run_conformance.py" --base-url "$URL" --label gate --scope "$SCOPE" --target "$KIND" \
    --out-dir "$WORK/conformance" > "$WORK/gate.out" 2>&1
  GATE=$?
  tail -12 "$WORK/gate.out"
  [ -f "$WORK/conformance/gate.result.json" ] && cp "$WORK/conformance/gate.result.json" "$RESULTS/$LABEL.gate.json"
else
  GATE=1
fi

echo "== [$LABEL] corpus comparison (scope $SCOPE: $RESOURCES)"
if reset; then
  if [ -n "$START_CORPUS_WORKER" ]; then
    DC="docker compose -f $ORACLE/compose.oracle.yml --env-file $ORACLE/.env.oracle --profile ratelimit"
    # record_corpus.py waits for the cascade it needs (the `settle` of the step that depends on it), so there is nothing to wait for here
    $DC start worker beat-worker > "$WORK/worker.log" 2>&1 || echo "oracle-v1: the Plane worker did not start; see $WORK/worker.log" >&2
  fi
  uv run --script "$H/record_corpus.py" --base-url "$URL" --only "$RESOURCES" --out-dir "$WORK/corpus" \
    --raw-dir "$WORK/corpus-raw" > "$WORK/record.out" 2>&1
  REC=$?
  if [ "$REC" -ne 0 ]; then
    echo "oracle-v1: the recording failed (exit $REC); the end of $WORK/record.out:" >&2
    tail -8 "$WORK/record.out" >&2
    CORPUS=1
  else
    uv run --script "$H/compare_corpus.py" --recorded "$WORK/corpus" --scope "$SCOPE" --target "$KIND" \
      --out "$RESULTS/$LABEL.corpus.json" > "$WORK/compare.out" 2>&1
    CORPUS=$?
    cat "$WORK/compare.out"
  fi
else
  CORPUS=1
fi

echo "== [$LABEL] the comparison's own controls"
uv run --script "$H/compare_corpus.py" --scope "$SCOPE" --self-test > "$WORK/self-test.out" 2>&1
SELF=$?
cat "$WORK/self-test.out"

if [ "$WHAT" = controls ]; then
  echo "== [$LABEL] must-fail controls (C0-C4)"
  uv run --script "$H/run_controls.py" --upstream "$URL" --scope "$SCOPE" --target "$KIND" --reset-script "$RESET" \
    --out "$RESULTS/$LABEL.controls.json" > "$WORK/controls.out" 2>&1
  CONTROLS=$?
  cat "$WORK/controls.out"
fi

echo "== [$LABEL] gate=$GATE corpus=$CORPUS self-test=$SELF${WHAT:+ controls=$CONTROLS}  (0 is a pass)"
if [ "$GATE" -eq 0 ] && [ "$CORPUS" -eq 0 ] && [ "$SELF" -eq 0 ] && [ "$CONTROLS" -eq 0 ]; then
  echo "VERDICT: PASS"
  exit 0
fi
echo "VERDICT: FAIL"
exit 1
