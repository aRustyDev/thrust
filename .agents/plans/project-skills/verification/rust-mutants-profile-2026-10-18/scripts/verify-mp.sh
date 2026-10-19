#!/bin/bash
# Verify the follow-up (mutants profile + hermetic default filter) against the REAL workspace on main.
SC=$SCRATCH
WT=$WORKTREE
LOG=$SC/verify-mp.log
export PATH=$HOME/.cargo/bin:$PATH
: > "$LOG"
cd "$WT" || exit 1
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
note() { printf '%s\n' "$*" >> "$LOG"; }

note "##### P1 manifest still parses; [profile.mutants] present"
run p1-metadata cargo metadata --no-deps --format-version 1 --quiet
run p1-grep grep -n -A3 'profile.mutants' Cargo.toml

note "##### P2 default profile and ci now select no tests"
run p2-list-default cargo nextest list --workspace --message-format oneline
run p2-run-default cargo nextest run --workspace
run p2-run-ci cargo nextest run --workspace --profile ci
run p2-intersect cargo nextest run --workspace -E 'binary(pg_adapter)'

note "##### P3 the opt-in route to the Docker-backed adapter tests through nextest"
run p3-list-optin cargo nextest list --workspace --ignore-default-filter -E 'binary(pg_adapter)' --message-format oneline
run p3-list-contract-optin cargo nextest list --workspace --ignore-default-filter -E 'package(thrust-contract-tests)' --message-format oneline

note "##### P4 cargo-mutants: harness crate excluded, profile active"
cargo mutants --list > "$SC/mp-mlist-after.txt" 2>> "$LOG"
printf 'list exit=%s\n' "$?" >> "$LOG"
run p4-count wc -l "$SC/mp-mlist-after.txt"
run p4-harness grep -c 'crates/thrust-contract-tests' "$SC/mp-mlist-after.txt"
run p4-per-crate sh -c "grep -o -E '^crates/[^/]+' $SC/mp-mlist-after.txt | sort | uniq -c"

note "##### P5 a mutation run on a crate with no tests of its own (thrust-contracts, 9 mutants)"
rm -rf mutants.out mutants.out.old
run p5-mutants cargo mutants -f crates/thrust-contracts/src/lib.rs
run p5-baseline-log sh -c "sed -n 1,40p mutants.out/log/baseline.log | cut -c1-220"
rm -rf mutants.out mutants.out.old

printf 'finished\n' >> "$LOG"
