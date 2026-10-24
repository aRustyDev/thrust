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
  # A pass run with --nocapture (the engine pass) prints `test x ... ` and the panic text, and only then `FAILED` on a
  # line of its own, so its failures are counted from cargo's own `test result: FAILED. N passed; M failed` line.
  if [ "$failed" -eq 0 ]; then
    failed="$(sed -n 's/^test result: FAILED\. [0-9]* passed; \([0-9]*\) failed.*/\1/p' "$log" | tail -1)"
    failed="${failed:-0}"
  fi
  if [ "$status" -eq 101 ] && [ "$failed" -ge 1 ]; then
    printf '%-34s KILLED   (%s test(s) failed)\n' "$label" "$failed" >>"$SUMMARY"
    grep -E '\.\.\. FAILED$|^test .* \.\.\. $' "$log" | sed 's/^/    /' >>"$SUMMARY"
  else
    printf '%-34s SURVIVED or did not run (exit=%s, failed tests=%s)\n' "$label" "$status" "$failed" >>"$SUMMARY"
    SURVIVED=1
  fi
}

# name:feature:pass. The CSRF switch (Q-0050.8) has a mutant for each direction: ignored when on (the main pass, which runs
# with enforcement on, must fail) and ignored when off (the `csrfoff` pass must fail).
for triple in \
  "ignore-tenant:mutant_ignore_tenant:main" \
  "duplicate-sequence-number:mutant_dup_sequence:main" \
  "no-activity-dedupe:mutant_no_activity_dedupe:main" \
  "ignore-revoked:mutant_ignore_revoked:main" \
  "skip-csrf:mutant_skip_csrf:main" \
  "csrf-switch-ignored-when-on:mutant_csrf_never:main" \
  "csrf-switch-ignored-when-off:mutant_csrf_always:csrfoff" \
  "unverified-email:mutant_unverified_email:main"; do
  name="${triple%%:*}"
  rest="${triple#*:}"
  feature="${rest%%:*}"
  pass="${rest##*:}"
  THRUST_SERVER_FEATURES="$feature" THRUST_LOGDIR="$OUT/$name" "$ROOT/scripts/contract.sh" native-memory "$pass" >"$OUT/$name.out" 2>&1
  status=$?
  kill_check "native-memory $pass + $name" "$status" "$OUT/$name/native-memory-$pass.test.log"
done

# The engine pass (spike 0002, `contract.sh <target> engine`): the 50-writer, 10k-create test against the duplicate-sequence
# mutant of the memory adapter, and the search tests against its case-sensitive-search mutant. Each must fail it: that is the
# control the issue names for the 50-writer test. (The SQLite adapter's own rows are archived with it, on the tag
# `spike/0002-worker-do`.)
for spec in \
  "native-memory:duplicate-sequence-number:mutant_dup_sequence" \
  "native-memory:case-sensitive-search:mutant_search_case_sensitive"; do
  target="${spec%%:*}"
  rest="${spec#*:}"
  name="${rest%%:*}"
  feature="${rest##*:}"
  THRUST_SERVER_FEATURES="$feature" THRUST_LOGDIR="$OUT/engine-$target-$name" "$ROOT/scripts/contract.sh" "$target" engine >"$OUT/engine-$target-$name.out" 2>&1
  status=$?
  kill_check "$target engine + $name" "$status" "$OUT/engine-$target-$name/$target-engine.test.log"
done
# The vacuity guard of the CSRF switch (Q-0050.8): a pass that is started against a server with the OTHER enforcement
# setting must be refused before a test runs (exit 5, and the message says csrf_enforce). Not a build mutant: the server
# is the real one, started with THRUST_SERVER_CSRF_ENFORCE, the knob only this script sets.
guard_check() {
  # $1 label, $2 exit status, $3 output file
  label="$1"; status="$2"; out="$3"
  if [ "$status" -eq 5 ] && grep -q 'VACUITY GUARD: csrf_enforce' "$out"; then
    printf '%-34s REFUSED  (exit 5, no test ran)\n' "$label" >>"$SUMMARY"
  else
    printf '%-34s NOT REFUSED (exit=%s)\n' "$label" "$status" >>"$SUMMARY"
    SURVIVED=1
  fi
}
THRUST_SERVER_CSRF_ENFORCE=off THRUST_LOGDIR="$OUT/guard-main-vs-off" "$ROOT/scripts/contract.sh" native-memory main >"$OUT/guard-main-vs-off.out" 2>&1
status=$?
guard_check "guard: main pass, server off" "$status" "$OUT/guard-main-vs-off.out"
THRUST_SERVER_CSRF_ENFORCE=on THRUST_LOGDIR="$OUT/guard-off-vs-on" "$ROOT/scripts/contract.sh" native-memory csrfoff >"$OUT/guard-off-vs-on.out" 2>&1
status=$?
guard_check "guard: csrfoff pass, server on" "$status" "$OUT/guard-off-vs-on.out"

# Postgres adapter mutant: recycle a connection that is still inside an abandoned transaction.
"$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-adapters-selfhosted \
  --features mutant_discard_noop --test pg_adapter -- an_abandoned_transaction model_postgres_adapter_matches >"$OUT/pg-discard.out" 2>&1
status=$?
kill_check "postgres adapter + discard-noop" "$status" "$OUT/pg-discard.out"

cat "$SUMMARY"
exit "$SURVIVED"
