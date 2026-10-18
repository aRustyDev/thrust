#!/bin/bash
# Final rust-test-app templates (after the review fixes): compile and run all four, then the class-branch control
# that the review found could not fail on the previous template.
SC=$SCRATCH
Z=$REPO/reference-workspace.zip
EX=$WORKTREE/.agents/plans/project-skills/artifacts/skills/rust-test-app/references/examples
R=$SC/ref/reference-workspace
LOG=$SC/verify-11.log
: > "$LOG"
cd "$R" || exit 1
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
note() { printf '%s\n' "$*" >> "$LOG"; }

cp "$EX/api_example.rs" crates/api/src/lib.rs
cp "$EX/ui_components_example.rs" crates/ui/src/lib.rs
cp "$EX/harness_lib.rs" crates/e2e/src/lib.rs
cp "$EX/e2e_harness_test.rs" crates/e2e/tests/e2e_harness.rs
touch crates/api/src/lib.rs crates/ui/src/lib.rs crates/e2e/src/lib.rs crates/e2e/tests/e2e_harness.rs

note "-- H1 final templates: default profile, e2e profile"
run h1-default cargo nextest run --workspace
run h2-e2e cargo nextest run --workspace --profile e2e

note "-- H3 the assertions that the review said the old control could not reach"
run h3-grep grep -n -E 'data-testid="entry-[12]"|class: if entry.done' crates/ui/src/lib.rs

note "-- H4 control: invert the class branch in PRODUCTION code only (swap done/open), expect FAIL"
perl -pi -e 's/class: if entry\.done \{ "done" \} else \{ "open" \}/class: if entry.done { "open" } else { "done" }/' crates/ui/src/lib.rs
touch crates/ui/src/lib.rs
run h4-grep grep -n 'class: if entry.done' crates/ui/src/lib.rs
run h4-inverted cargo nextest run -p ui --no-fail-fast

note "-- H5 control on the PREVIOUS template (the one the review said cannot fail): same inversion"
unzip -o -q "$Z" reference-workspace/crates/ui/src/lib.rs -d "$SC/ref"
perl -pi -e 's/class: if todo\.done \{ "done" \} else \{ "open" \}/class: if todo.done { "open" } else { "done" }/' crates/ui/src/lib.rs
touch crates/ui/src/lib.rs
run h5-grep grep -n 'class: if todo.done' crates/ui/src/lib.rs
run h5-original-inverted cargo nextest run -p ui --no-fail-fast

note "-- restore the archive originals"
unzip -o -q "$Z" reference-workspace/crates/api/src/lib.rs reference-workspace/crates/ui/src/lib.rs reference-workspace/crates/e2e/src/lib.rs reference-workspace/crates/e2e/tests/e2e_harness.rs -d "$SC/ref"
touch crates/api/src/lib.rs crates/ui/src/lib.rs crates/e2e/src/lib.rs crates/e2e/tests/e2e_harness.rs
run r1-restored cargo nextest run --workspace
printf 'finished\n' >> "$LOG"
