#!/bin/bash
# W6 redone: the probe's flaky test file failed the cargo-mutants baseline (ZZ_MARKER_DIR unset). Remove it, then rerun.
SC=$SCRATCH
W=$SC/ws-verify
LOG=$SC/verify-ws2.log
export PATH=$HOME/.cargo/bin:$PATH
: > "$LOG"
cd "$W" || exit 1
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
rm -f crates/probe/tests/flaky.rs
rm -rf mutants.out mutants.out.old
run x0-baseline cargo nextest run
cargo mutants --list > "$SC/ws-mlist-with.txt" 2>> "$LOG"
printf 'list with config exit=%s\n' "$?" >> "$LOG"
run x1-files-with grep -o -E '^[^:]+' "$SC/ws-mlist-with.txt"
mv .cargo/mutants.toml "$SC/ws-mutants.toml.bak"
cargo mutants --list > "$SC/ws-mlist-without.txt" 2>> "$LOG"
printf 'list without config exit=%s\n' "$?" >> "$LOG"
mv "$SC/ws-mutants.toml.bak" .cargo/mutants.toml
run x2-files-without grep -o -E '^[^:]+' "$SC/ws-mlist-without.txt"
run x3-mutants cargo mutants -f crates/probe/src/lib.rs
run x4-counts wc -l mutants.out/caught.txt mutants.out/missed.txt mutants.out/unviable.txt mutants.out/timeout.txt
run x5-argv grep -n -E 'cargo-profile|--profile' mutants.out/log/baseline.log
printf 'finished\n' >> "$LOG"
