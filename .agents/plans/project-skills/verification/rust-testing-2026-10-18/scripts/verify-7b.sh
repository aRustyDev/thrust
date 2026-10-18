#!/bin/bash
# Proper control: mutate ONLY the production function in the renamed ui example, not the assertions.
SC=$SCRATCH
Z=$REPO/reference-workspace.zip
EX=$WORKTREE/.agents/plans/project-skills/artifacts/skills/rust-test-app/references/examples
R=$SC/ref/reference-workspace
LOG=$SC/verify-7b.log
: > "$LOG"
cd "$R" || exit 1
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
cp "$EX/ui_components_example.rs" crates/ui/src/lib.rs
perl -pi -e 's/^(\s*1 => )"1 item left"/$1"1 items left"/' crates/ui/src/lib.rs
touch crates/ui/src/lib.rs
run g1-grep grep -n 'item' crates/ui/src/lib.rs
run g2-ui-prod-mutated cargo nextest run -p ui --no-fail-fast
unzip -o -q "$Z" reference-workspace/crates/ui/src/lib.rs -d "$SC/ref"
touch crates/ui/src/lib.rs
run g3-restored cargo nextest run -p ui
printf 'finished\n' >> "$LOG"
