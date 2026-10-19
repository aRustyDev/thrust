#!/bin/sh
# Everything the results file cites, one serial run, one summary:
#   0. the hermetic tests                 (cargo nextest run, default profile: thrust-editor-doc today)
#   1. the 16-cell contract matrix        (scripts/contract-all.sh)
#   2. the mutation controls              (scripts/mutants.sh)   -> every mutant must be KILLED
#   3. the dependency-graph check         (scripts/check-deps.sh)
#   4. the testcontainers adapter tests   (thrust-adapters-selfhosted/tests/pg_adapter.rs)
#   5. the port-shape and sqlx probes     (probes/*/run.sh)
# Serial on purpose (shared wrangler build dir, shared ports). Needs Docker.
#
#   scripts/verify-all.sh        -> target/verify/summary.txt
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/verify"
mkdir -p "$OUT"
SUMMARY="$OUT/summary.txt"
: >"$SUMMARY"
FAILED=0

step() {
  name="$1"; shift
  "$@" >"$OUT/$name.log" 2>&1
  status=$?
  if [ "$status" -eq 0 ]; then verdict=OK; else verdict="FAIL(exit $status)"; FAILED=1; fi
  printf '%-28s %s\n' "$name" "$verdict" >>"$SUMMARY"
}

step hermetic-tests "$ROOT/scripts/rs" cargo nextest run --manifest-path "$ROOT/Cargo.toml" --workspace
step contract-matrix "$ROOT/scripts/contract-all.sh"
step mutants "$ROOT/scripts/mutants.sh"
step check-deps "$ROOT/scripts/check-deps.sh"
step pg-adapter-tests "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-adapters-selfhosted --test pg_adapter
step probe-send-shape "$ROOT/probes/send-shape/run.sh"
# the sqlx probe is EXPECTED to fail to compile; the step only checks that the script ran
"$ROOT/probes/sqlx-wasm/run.sh" >"$OUT/probe-sqlx-wasm.log" 2>&1
printf '%-28s %s\n' "probe-sqlx-wasm (informative)" "ran" >>"$SUMMARY"

cat "$SUMMARY"
cat "$ROOT/target/contract-logs/summary.txt"
cat "$ROOT/target/mutants/summary.txt"
exit "$FAILED"
