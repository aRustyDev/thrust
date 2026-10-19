#!/bin/bash
# FINAL check: the committed config files exactly as they will ship, in a scratch workspace with a probe crate and
# NO [profile.mutants] (the first crate's PR adds it). Also confirms the AGENTS.md commands.
SC=$SCRATCH
WT=$WORKTREE
W=$SC/ws-final
LOG=$SC/verify-ws4.log
export PATH=$HOME/.cargo/bin:$PATH
: > "$LOG"
rm -rf "$W"
mkdir -p "$W/.config" "$W/.cargo" "$W/crates/probe/src"
cp "$WT/.config/nextest.toml" "$W/.config/nextest.toml"
cp "$WT/.cargo/mutants.toml" "$W/.cargo/mutants.toml"
cat > "$W/Cargo.toml" <<'EOF'
[workspace]
resolver = "3"
members = ["crates/probe"]

[workspace.package]
edition = "2021"
version = "0.0.0"
license = "AGPL-3.0-only"
publish = false
EOF
cat > "$W/crates/probe/Cargo.toml" <<'EOF'
[package]
name = "probe"
edition.workspace = true
version.workspace = true
license.workspace = true
publish.workspace = true
EOF
cat > "$W/crates/probe/src/lib.rs" <<'EOF'
pub fn add(a: u32, b: u32) -> u32 { a + b }
pub fn is_zero(n: usize) -> bool { n == 0 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn plain_adds() { assert_eq!(add(1, 2), 3); }
    #[test] fn prop_add_commutes() { for a in 0..10 { for b in 0..10 { assert_eq!(add(a, b), add(b, a)); } } }
    #[test] fn model_is_zero() { assert!(is_zero(0)); assert!(!is_zero(1)); }
    #[test] fn oracle_add_matches() { assert_eq!(add(2, 2), 4); }
}
EOF
cd "$W" || exit 1
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
run z0-grep-no-profile grep -c 'profile.mutants' Cargo.toml
run z1-default cargo nextest run
run z2-ci cargo nextest run --profile ci --workspace
run z3-deep env PROPTEST_CASES=4096 cargo nextest run --profile deep
run z4-oracle cargo nextest run --profile oracle
run z5-category cargo nextest run -E 'test(/\bprop_/)'
run z6-empty-filter cargo nextest run -E 'test(/\bnope_/)'
run z7-doc cargo test --doc --workspace
run z8-mutants cargo mutants -f crates/probe/src/lib.rs
run z9-counts wc -l mutants.out/caught.txt mutants.out/missed.txt
run za-notice sh -c "cargo nextest run 2>&1 | grep -c 'recommends nextest'"
printf 'finished\n' >> "$LOG"
