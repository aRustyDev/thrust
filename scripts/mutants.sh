#!/bin/sh
# Mutation controls: prove the contract suite can FAIL, i.e. that its "X is rejected" assertions
# observe the thing they claim to test. Each mutant is a deliberately broken build; the suite must
# fail on it (exit 101 and at least one test reported FAILED). A mutant that survives means some
# assertion is vacuous.
#
#   scripts/mutants.sh
set -u
# A counterexample found against a deliberately broken build is not a regression of the real code: do not let the
# generated tests (crates/thrust-contract-tests/tests/model) save it into the source tree.
export PROPTEST_DISABLE_FAILURE_PERSISTENCE=1
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/mutants"
mkdir -p "$OUT"
SUMMARY="$OUT/summary.txt"
: >"$SUMMARY"
SURVIVED=0

kill_check() {
  # $1 label, $2 exit status, $3 log file with test output
  label="$1"; status="$2"; log="$3"
  failed="$(grep -c '\.\.\. FAILED$' "$log")"
  if [ "$status" -eq 101 ] && [ "$failed" -ge 1 ]; then
    printf '%-34s KILLED   (%s test(s) failed)\n' "$label" "$failed" >>"$SUMMARY"
    grep '\.\.\. FAILED$' "$log" | sed 's/^/    /' >>"$SUMMARY"
  else
    printf '%-34s SURVIVED or did not run (exit=%s, failed tests=%s)\n' "$label" "$status" "$failed" >>"$SUMMARY"
    SURVIVED=1
  fi
}

for pair in \
  "ignore-tenant:mutant_ignore_tenant" \
  "duplicate-sequence-number:mutant_dup_sequence" \
  "no-activity-dedupe:mutant_no_activity_dedupe"; do
  name="${pair%%:*}"
  feature="${pair##*:}"
  THRUST_SERVER_FEATURES="$feature" THRUST_LOGDIR="$OUT/$name" "$ROOT/scripts/contract.sh" native-memory main >"$OUT/$name.out" 2>&1
  status=$?
  kill_check "native-memory + $name" "$status" "$OUT/$name/native-memory-main.test.log"
done

# Postgres adapter mutant: recycle a connection that is still inside an abandoned transaction.
"$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-adapters-selfhosted \
  --features mutant_discard_noop --test pg_adapter -- an_abandoned_transaction model_postgres_adapter_matches >"$OUT/pg-discard.out" 2>&1
status=$?
kill_check "postgres adapter + discard-noop" "$status" "$OUT/pg-discard.out"

cat "$SUMMARY"
exit "$SURVIVED"
