#!/bin/bash
SC=$SCRATCH
R=$SC/ref/reference-workspace
LOG=$SC/verify-6.log
export PATH=$HOME/.cargo/bin:$PATH
: > "$LOG"
cd "$R" || exit 1
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
rm -rf mutants.out mutants.out.old
cargo mutants --list > "$SC/mlist-with.txt" 2>> "$LOG"
printf 'with config: list exit=%s\n' "$?" >> "$LOG"
mv .cargo/mutants.toml "$SC/mutants.toml.bak"
cargo mutants --list > "$SC/mlist-without.txt" 2>> "$LOG"
printf 'without config: list exit=%s\n' "$?" >> "$LOG"
mv "$SC/mutants.toml.bak" .cargo/mutants.toml
run c1-with grep -c -E 'crates/e2e|fuzz_harness|test_support|tests/common' "$SC/mlist-with.txt"
run c1-without grep -c -E 'crates/e2e|fuzz_harness|test_support|tests/common' "$SC/mlist-without.txt"
run c1-without-files grep -o -E '(crates/e2e[^:]*|[^: ]*fuzz_harness[^:]*|[^: ]*test_support[^:]*)' "$SC/mlist-without.txt"
run c2-run cargo mutants -f crates/core/src/store.rs
run c2-ls ls mutants.out
run c2-ls-log ls mutants.out/log
run c2-outcomes head -c 600 mutants.out/outcomes.json
run c2-diff ls mutants.out/diff
rm -rf mutants.out mutants.out.old
printf 'finished\n' >> "$LOG"
