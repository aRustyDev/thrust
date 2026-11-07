#!/usr/bin/env bash
# Run the whole v1 oracle pipeline against a FRESH pinned stack, then (optionally) tear it down.
#
#   oracle/scripts/run-all.sh [--refreeze] [--keep] [--no-build]
#
# Modes
#   default      CHECK: regenerate every artefact into oracle/.work/ and diff against the committed
#                ones. A difference means the pinned oracle is not reproducible (or drifted).
#   --refreeze   write the committed artefacts (schema/, corpus/, results/). Use deliberately.
#   --keep       leave the stack running (default: `docker compose down`, keeping images, dropping volumes)
#   --no-build   reuse an existing plane-oracle-api:local image
#
# Each stage logs to oracle/.work/logs/<stage>.log and its wall-clock lands in oracle/.timings.log.
# Exit status is the status of the FIRST failing stage; nothing is piped through tee (a pipe would
# report tee's status and hide a failed stage).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
REFREEZE=0; KEEP=0; BUILD=1
for arg in "$@"; do
  case "$arg" in
    --refreeze) REFREEZE=1 ;;
    --keep) KEEP=1 ;;
    --no-build) BUILD=0 ;;
    *) echo "unknown argument: $arg" >&2; exit 64 ;;
  esac
done

DC=(docker compose -f oracle/compose.oracle.yml --env-file oracle/.env.oracle)
TIMED=(oracle/scripts/timed.sh)
LOGS=oracle/.work/logs
mkdir -p "$LOGS" oracle/.work
: > oracle/.timings.log

API=http://127.0.0.1:18080
RL=http://127.0.0.1:18081
if [ "$REFREEZE" = 1 ]; then
  SCHEMA=oracle/schema/openapi.v1.json
  CORPUS=oracle/corpus
  RESULTS=oracle/results
else
  SCHEMA=oracle/.work/live/openapi.v1.json
  CORPUS=oracle/.work/live/corpus
  RESULTS=oracle/.work/live/results
fi
mkdir -p "$(dirname "$SCHEMA")" "$CORPUS" "$RESULTS"

stage() {  # stage <name> <command...>   (runs, logs, aborts with the command's own status)
  local name="$1"; shift
  echo "== $name"
  if ! "${TIMED[@]}" "$name" "$@" > "$LOGS/$name.log" 2>&1; then
    echo "STAGE FAILED: $name (see $LOGS/$name.log)" >&2
    tail -25 "$LOGS/$name.log" >&2
    exit 1
  fi
}

extract() {  # extract <BEGIN> <END> <infile> <outfile>
  sed -n "/$1/,/$2/p" "$3" | sed '1d;$d' > "$4"
}

# --- 0. is this checkout the pinned Plane? clean slate -----------------------------------------------
stage verify-pin oracle/scripts/verify-pin.sh
stage verify-pin-selftest oracle/scripts/verify-pin.sh --self-test
stage down-fresh "${DC[@]}" --profile ratelimit down -v --remove-orphans

# --- 1. build, 2. cold start --------------------------------------------------------------------
if [ "$BUILD" = 1 ]; then
  stage build-api-image "${DC[@]}" build migrator
fi
stage cold-start "${DC[@]}" up -d --wait --wait-timeout 300

# --- 3. seed (synthetic data, through the Django shell inside the container) ----------------------
stage seed "${DC[@]}" exec -T api python manage.py shell -c "exec(open('/oracle/seed.py').read())"
extract ORACLE_MANIFEST_BEGIN ORACLE_MANIFEST_END "$LOGS/seed.log" oracle/.work/live-manifest.json
if [ "$REFREEZE" = 1 ]; then cp oracle/.work/live-manifest.json oracle/corpus/seed-manifest.json; fi
MANIFEST=oracle/corpus/seed-manifest.json
if [ "$REFREEZE" != 1 ]; then
  # CHECK mode: the seed is deterministic, so the manifest must equal the committed one.
  stage manifest-reproducible cmp oracle/.work/live-manifest.json "$MANIFEST"
fi

# --- 3b. did the woven OIDC code survive the upstream merge? (probe the RUNNING stack) -------------
stage oidc-probe uv run oracle/harness/probe_oidc.py "$API"
stage save-oidc-probe cp "$LOGS/oidc-probe.log" "$RESULTS/oidc-probe.txt"

# --- 4. schema (served by the pinned instance) + determinism --------------------------------------
stage schema-fetch curl -fsS -o "$SCHEMA" "$API/api/schema/?format=json"
stage schema-fetch-again curl -fsS -o oracle/.work/schema.second.json "$API/api/schema/?format=json"
stage schema-deterministic cmp "$SCHEMA" oracle/.work/schema.second.json
if [ "$REFREEZE" != 1 ]; then
  stage schema-matches-committed cmp "$SCHEMA" oracle/schema/openapi.v1.json
fi

# --- 5. Q-0122.1: does the schema cover every routed v1 operation? --------------------------------
stage urlconf "${DC[@]}" exec -T api python manage.py shell -c "exec(open('/oracle/urlconf_ops.py').read())"
extract ORACLE_URLCONF_BEGIN ORACLE_URLCONF_END "$LOGS/urlconf.log" oracle/.work/urlconf.json
stage coverage uv run oracle/harness/coverage_report.py --urlconf oracle/.work/urlconf.json \
  --schema "$SCHEMA" --out "$RESULTS/v1-coverage.json" --self-test
stage conventions uv run oracle/harness/schema_conventions.py "$SCHEMA" --out "$RESULTS/schema-conventions.json"

# --- 6. corpus (on the pristine seed, before the fuzzer mutates it) --------------------------------
stage ratelimit-service-up "${DC[@]}" --profile ratelimit up -d --wait --wait-timeout 120 api-ratelimit
stage normalise-selftest uv run oracle/harness/normalise.py --self-test
stage corpus uv run oracle/harness/record_corpus.py --base-url "$API" --ratelimit-url "$RL" \
  --manifest "$MANIFEST" --out-dir "$CORPUS" --raw-dir oracle/.work/corpus-raw
stage normalisation-erases-volatiles uv run oracle/harness/check_normalisation.py oracle/.work/corpus-raw "$CORPUS"
if [ "$REFREEZE" != 1 ]; then
  # CHECK mode: a second, independent recording on a fresh stack must equal the committed corpus.
  stage corpus-reproducible diff -r oracle/corpus "$CORPUS" --exclude=seed-manifest.json
  stage coverage-reproducible cmp oracle/results/v1-coverage.json "$RESULTS/v1-coverage.json"
  stage conventions-reproducible cmp oracle/results/schema-conventions.json "$RESULTS/schema-conventions.json"
fi

# --- 7. conformance gate, then the must-fail controls ----------------------------------------------
# The gate fuzzes PATCH/POST and leaves the database different. Every gate and control run therefore
# starts from the SAME snapshot (pristine seed + corpus), or a control would be compared with a
# baseline that ran on different data. worker/beat stay stopped so nothing mutates data underneath.
stage db-snapshot oracle/scripts/db-state.sh snapshot
stage db-restore-for-gate oracle/scripts/db-state.sh restore
stage conformance-gate uv run oracle/harness/run_conformance.py --base-url "$API" --label gate \
  --schema "$SCHEMA" --manifest "$MANIFEST" --out-dir oracle/.work/conformance
stage save-gate-result cp oracle/.work/conformance/gate.result.json "$RESULTS/conformance-gate.json"
stage controls uv run oracle/harness/run_controls.py --upstream "$API" --schema "$SCHEMA" \
  --manifest "$MANIFEST" --out "$RESULTS/controls.json" --reset-script oracle/scripts/db-state.sh

# --- 8. schema diff job (oasdiff) -----------------------------------------------------------------
stage schema-diff oracle/scripts/schema_diff.sh "$SCHEMA" "$RESULTS/schema-diff.json"

# --- teardown ---------------------------------------------------------------------------------------
if [ "$KEEP" = 0 ]; then
  stage down-final "${DC[@]}" --profile ratelimit down -v --remove-orphans
fi
echo "== ALL STAGES PASSED"
cat oracle/.timings.log
