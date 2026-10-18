#!/bin/bash
# Compile and run the rust-test-app skill's RENAMED examples inside the scratchpad copy of the zip workspace.
SC=$SCRATCH
Z=$REPO/reference-workspace.zip
EX=$WORKTREE/.agents/plans/project-skills/artifacts/skills/rust-test-app/references/examples
R=$SC/ref/reference-workspace
LOG=$SC/verify-7.log
: > "$LOG"
cd "$R" || exit 1
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
restore() { unzip -o -q "$Z" "$@" -d "$SC/ref" >> "$LOG" 2>&1; for f in "$@"; do touch "$SC/ref/$f"; done; }

cp "$EX/api_example.rs" crates/api/src/lib.rs
cp "$EX/ui_components_example.rs" crates/ui/src/lib.rs
cp "$EX/harness_lib.rs" crates/e2e/src/lib.rs
cp "$EX/e2e_harness_test.rs" crates/e2e/tests/e2e_harness.rs
touch crates/api/src/lib.rs crates/ui/src/lib.rs crates/e2e/src/lib.rs crates/e2e/tests/e2e_harness.rs

run a1-default cargo nextest run --workspace
run a2-e2e cargo nextest run --workspace --profile e2e
run a3-doc cargo test --doc --workspace

printf '\n-- CONTROL: break the renamed ui example (plural label) and the renamed e2e deep link expectation\n' >> "$LOG"
perl -pi -e 's/"1 item left"/"1 items left"/' crates/ui/src/lib.rs
touch crates/ui/src/lib.rs
run c1-ui-mutated cargo nextest run -p ui
cp "$EX/ui_components_example.rs" crates/ui/src/lib.rs
touch crates/ui/src/lib.rs
perl -pi -e 's/status\(\)\.as_u16\(\), 200/status().as_u16(), 404/' crates/e2e/tests/e2e_harness.rs
touch crates/e2e/tests/e2e_harness.rs
run c2-e2e-mutated cargo nextest run --workspace --profile e2e

restore reference-workspace/crates/api/src/lib.rs reference-workspace/crates/ui/src/lib.rs reference-workspace/crates/e2e/src/lib.rs reference-workspace/crates/e2e/tests/e2e_harness.rs
run r1-restored cargo nextest run --workspace
printf 'finished\n' >> "$LOG"
