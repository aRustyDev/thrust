#!/bin/bash
# Verify the LIVE workspace files (Cargo.toml, .config/nextest.toml, .cargo/mutants.toml) in a scratch workspace.
SC=$SCRATCH
WT=$WORKTREE
W=$SC/ws-verify
LOG=$SC/verify-ws.log
export PATH=$HOME/.cargo/bin:$PATH
: > "$LOG"
rm -rf "$W"
mkdir -p "$W/.config" "$W/.cargo" "$W/crates/probe/src" "$W/crates/probe/tests"
cp "$WT/Cargo.toml" "$W/Cargo.toml"
cp "$WT/.config/nextest.toml" "$W/.config/nextest.toml"
cp "$WT/.cargo/mutants.toml" "$W/.cargo/mutants.toml"
cd "$W" || exit 1

run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
note() { printf '%s\n' "$*" >> "$LOG"; }

note "##### W1 the workspace AS IT WILL SHIP (members = [], no crates)"
run w1-metadata cargo metadata --no-deps --format-version 1 --quiet
run w1-build cargo build
run w1-nextest-run cargo nextest run
run w1-nextest-ci cargo nextest run --profile ci
run w1-mutants-list cargo mutants --list
run w1-test-doc cargo test --doc --workspace

note "##### W2 add a throwaway probe crate (scratch copy only)"
sed -i '' 's/members = \[\]/members = ["crates\/probe"]/' Cargo.toml
cat > crates/probe/Cargo.toml <<'EOF'
[package]
name = "probe"
edition.workspace = true
version.workspace = true
license.workspace = true
publish.workspace = true
EOF
cat > crates/probe/src/lib.rs <<'EOF'
pub mod fuzz_harness;
pub fn add(a: u32, b: u32) -> u32 { a + b }
pub fn is_zero(n: usize) -> bool { n == 0 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn plain_adds() { assert_eq!(add(1, 2), 3); }
    #[test] fn prop_add_commutes() { for a in 0..10 { for b in 0..10 { assert_eq!(add(a, b), add(b, a)); } } }
    #[test] fn model_is_zero() { assert!(is_zero(0)); assert!(!is_zero(1)); }
    #[test] fn oracle_add_matches() { assert_eq!(add(2, 2), 4); }
    #[test] fn parity_add_fixture() { assert_eq!(add(0, 0), 0); }
    #[test] fn fuzz_replay_nothing() { assert!(crate::fuzz_harness::check(&[1, 2, 3])); }
}
EOF
cat > crates/probe/src/fuzz_harness.rs <<'EOF'
pub fn check(data: &[u8]) -> bool { data.len() > 2 }
EOF
cat > crates/probe/tests/flaky.rs <<'EOF'
use std::path::PathBuf;
fn first_attempt(name: &str) -> bool {
    let dir = PathBuf::from(std::env::var("ZZ_MARKER_DIR").expect("ZZ_MARKER_DIR"));
    let marker = dir.join(name);
    if marker.exists() { false } else { std::fs::write(&marker, b"x").unwrap(); true }
}
#[test] fn prop_flaky_probe() { assert!(!first_attempt("prop"), "first attempt fails on purpose"); }
#[test] fn plain_flaky_probe() { assert!(!first_attempt("plain"), "first attempt fails on purpose"); }
EOF

note "##### W3 profile selection (lists, one per profile)"
run w3-list-default cargo nextest list --profile default --message-format oneline -E 'not test(/flaky/)'
run w3-list-oracle cargo nextest list --profile oracle --message-format oneline -E 'not test(/flaky/)'
run w3-list-deep cargo nextest list --profile deep --message-format oneline -E 'not test(/flaky/)'
run w3-list-ci cargo nextest list --profile ci --message-format oneline -E 'not test(/flaky/)'
run w3-list-mutants cargo nextest list --profile mutants --message-format oneline -E 'not test(/flaky/)'
run w3-list-e2e-absent cargo nextest list --profile e2e

note "##### W4 runs"
run w4-default cargo nextest run -E 'not test(/flaky/)'
run w4-oracle cargo nextest run --profile oracle
run w4-deep cargo nextest run --profile deep -E 'not test(/flaky/)'
run w4-filter-intersect cargo nextest run -E 'test(/\boracle_/)'

note "##### W5 retry behaviour with the shipped config (flaky probes fail once, then pass)"
d1=$(mktemp -d); d2=$(mktemp -d); d3=$(mktemp -d); d4=$(mktemp -d)
run w5a-prop-retries env ZZ_MARKER_DIR=$d1 cargo nextest run --test flaky --retries 2 -E 'test(prop_flaky_probe)'
run w5b-prop-flaky-result-pass env ZZ_MARKER_DIR=$d2 cargo nextest run --test flaky --retries 2 --flaky-result pass -E 'test(prop_flaky_probe)'
run w5c-plain-retries env ZZ_MARKER_DIR=$d3 cargo nextest run --test flaky --retries 2 -E 'test(plain_flaky_probe)'
run w5d-prop-no-retries env ZZ_MARKER_DIR=$d4 cargo nextest run --test flaky -E 'test(prop_flaky_probe)'

note "##### W6 cargo-mutants with the shipped .cargo/mutants.toml"
rm -rf mutants.out mutants.out.old
cargo mutants --list > "$SC/ws-mlist-with.txt" 2>> "$LOG"
printf 'list with config exit=%s\n' "$?" >> "$LOG"
run w6a-files grep -o -E '^[^:]+' "$SC/ws-mlist-with.txt"
mv .cargo/mutants.toml "$SC/ws-mutants.toml.bak"
cargo mutants --list > "$SC/ws-mlist-without.txt" 2>> "$LOG"
printf 'list without config exit=%s\n' "$?" >> "$LOG"
mv "$SC/ws-mutants.toml.bak" .cargo/mutants.toml
run w6b-without-files grep -o -E '^[^:]+' "$SC/ws-mlist-without.txt"
run w6c-run cargo mutants -f crates/probe/src/lib.rs
run w6d-counts wc -l mutants.out/caught.txt mutants.out/missed.txt mutants.out/unviable.txt mutants.out/timeout.txt
run w6e-argv grep -n -E 'cargo-profile|--profile' mutants.out/log/baseline.log

note "##### DONE"
printf 'finished\n' >> "$LOG"
