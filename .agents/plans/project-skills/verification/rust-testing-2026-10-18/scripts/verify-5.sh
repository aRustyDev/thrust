#!/bin/bash
# cargo-mutants 27.1.0 controls against the zip's reference workspace (scratchpad copy only).
SC=$SCRATCH
Z=$REPO/reference-workspace.zip
R=$SC/ref/reference-workspace
LOG=$SC/verify-5.log
export PATH=$HOME/.cargo/bin:$PATH
: > "$LOG"
cd "$R" || exit 1

section() { printf '\n##### %s\n' "$1" >> "$LOG"; }
note() { printf '%s\n' "$*" >> "$LOG"; }
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
restore() { unzip -o -q "$Z" "$@" -d "$SC/ref" >> "$LOG" 2>&1; for f in "$@"; do touch "$SC/ref/$f"; done; }
clean() { rm -rf mutants.out mutants.out.old; }
clean

section "M0 version and mutant list for store.rs"
run m0-version cargo mutants --version
run m1-list cargo mutants --list -f crates/core/src/store.rs

section "M1 exclude_globs: whole-workspace list must not name e2e, fuzz_harness, test_support, tests/common"
cargo mutants --list > "$SC/mlist.txt" 2>> "$LOG"
note "list exit=$?"
run m1b-count-excluded grep -c -E 'crates/e2e|fuzz_harness|test_support|tests/common' "$SC/mlist.txt"
run m1c-count-core grep -c 'crates/core/src/frame.rs' "$SC/mlist.txt"
run m1d-files grep -o -E '^[^:]+' "$SC/mlist.txt"

section "M2 default (temp-copy) run on store.rs: expect exit 0, 18 caught"
run m2-default cargo mutants -f crates/core/src/store.rs
run m2-counts wc -l mutants.out/caught.txt mutants.out/missed.txt mutants.out/unviable.txt mutants.out/timeout.txt
run m2-baseline-log head -12 mutants.out/log/baseline.log
clean

section "M5 --in-place trap: seeds saved into the tree (no env) vs not (with env)"
run m5a-before find crates -name '*proptest-regressions*'
run m5b-inplace cargo mutants --in-place -f crates/core/src/store.rs
run m5c-after find crates -name '*proptest-regressions*'
run m5c-cat cat crates/core/tests/model.proptest-regressions
rm -f crates/core/tests/model.proptest-regressions
clean
run m5d-inplace-env env PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 cargo mutants --in-place -f crates/core/src/store.rs
run m5e-after find crates -name '*proptest-regressions*'
run m5f-sha shasum crates/core/src/store.rs
clean

section "M3 which assertion kills the is_empty mutants? (in-place, persistence disabled)"
note "-- M3a: remove only the state-machine invariant (model.rs lines 139-140)"
perl -ni -e 'print unless $. == 139 || $. == 140' crates/core/tests/model.rs
run m3a-run env PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 cargo mutants --in-place -f crates/core/src/store.rs
run m3a-missed cat mutants.out/missed.txt
restore reference-workspace/crates/core/tests/model.rs
clean
note "-- M3b: remove only the lockstep assertion (model.rs line 45)"
perl -ni -e 'print unless $. == 45' crates/core/tests/model.rs
run m3b-run env PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 cargo mutants --in-place -f crates/core/src/store.rs
run m3b-missed cat mutants.out/missed.txt
restore reference-workspace/crates/core/tests/model.rs
clean
note "-- M3c: remove both"
perl -ni -e 'print unless $. == 45 || $. == 139 || $. == 140' crates/core/tests/model.rs
run m3c-run env PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 cargo mutants --in-place -f crates/core/src/store.rs
run m3c-missed cat mutants.out/missed.txt
note "-- M4: restore the tests, then --iterate must re-test only the missed ones and catch them"
restore reference-workspace/crates/core/tests/model.rs
run m4-iterate env PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 cargo mutants --in-place --iterate -f crates/core/src/store.rs
clean

section "M6 --in-diff scopes the run to changed code"
cp crates/core/src/store.rs "$SC/store.orig"
perl -pi -e 's/self\.entries\.is_empty\(\)/self.entries.len() == 0/' crates/core/src/store.rs
diff -u -L a/crates/core/src/store.rs -L b/crates/core/src/store.rs "$SC/store.orig" crates/core/src/store.rs > "$SC/pr.diff"
note "diff exit=$? (1 = files differ, expected)"
run m6a-diff cat "$SC/pr.diff"
run m6b-list-in-diff cargo mutants --list --in-diff "$SC/pr.diff"
run m6c-run env PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 cargo mutants --in-place --in-diff "$SC/pr.diff"
restore reference-workspace/crates/core/src/store.rs
run m6d-sha shasum crates/core/src/store.rs
clean

section "M7 exit code when the baseline (unmutated) tests already fail: expect 4"
perl -pi -e 's/tag: bytes\[0\],/tag: bytes[0].wrapping_add(1),/' crates/core/src/frame.rs
run m7-baseline-fails env PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 cargo mutants --in-place -f crates/core/src/store.rs
restore reference-workspace/crates/core/src/frame.rs
clean

section "DONE"
note "finished"
