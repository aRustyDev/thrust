#!/bin/bash
# Isolate the proptest persistence claim: does DEFAULT persistence drop counterexamples for tests/ files?
SC=$SCRATCH
R=$SC/ref/reference-workspace
LOG=$SC/verify-4.log
: > "$LOG"
cd "$R" || exit 1
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
rm -f crates/core/tests/zz_persist.proptest-regressions
cp "$SC/probes/zz_persist.rs" crates/core/tests/zz_persist.rs
run v4-clean-before find crates/core -name '*proptest-regressions*'
run v4a-default-only cargo nextest run -p core-lib --test zz_persist -E 'test(prop_default_persistence)'
run v4a-find find crates/core -name '*proptest-regressions*'
run v4a-cat cat crates/core/tests/zz_persist.proptest-regressions
rm -f crates/core/tests/zz_persist.proptest-regressions
printf '\n-- default persistence + PROPTEST_DISABLE_FAILURE_PERSISTENCE=1\n' >> "$LOG"
run v4b-default-env env PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 cargo nextest run -p core-lib --test zz_persist -E 'test(prop_default_persistence)'
run v4b-find find crates/core -name '*proptest-regressions*'
rm -f crates/core/tests/zz_persist.rs crates/core/tests/zz_persist.proptest-regressions
run v4-clean-after find crates/core -name '*zz_*'
printf 'finished\n' >> "$LOG"
