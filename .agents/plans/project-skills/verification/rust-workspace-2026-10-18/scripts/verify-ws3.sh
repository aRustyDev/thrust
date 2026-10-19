#!/bin/bash
# What happens when .cargo/mutants.toml says profile = "mutants" but the root Cargo.toml has no [profile.mutants]?
SC=$SCRATCH
W=$SC/ws-verify
LOG=$SC/verify-ws3.log
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
cp Cargo.toml "$SC/ws-Cargo.toml.bak"
cp .cargo/mutants.toml "$SC/ws-mutants.toml.keep"
rm -rf mutants.out mutants.out.old

# (1) profile line present, Cargo profile missing
perl -0pi -e 's/\[profile\.mutants\].*\z//s' Cargo.toml
run y0-cargo-toml grep -c 'profile.mutants' Cargo.toml
run y1-missing-profile cargo mutants -f crates/probe/src/lib.rs
rm -rf mutants.out mutants.out.old

# (2) the profile line commented out, Cargo profile still missing
perl -pi -e 's/^profile = "mutants"/# profile = "mutants"/' .cargo/mutants.toml
run y2-grep grep -n 'profile = ' .cargo/mutants.toml
run y3-commented-profile cargo mutants -f crates/probe/src/lib.rs
run y4-counts wc -l mutants.out/caught.txt mutants.out/missed.txt
rm -rf mutants.out mutants.out.old

cp "$SC/ws-Cargo.toml.bak" Cargo.toml
cp "$SC/ws-mutants.toml.keep" .cargo/mutants.toml
printf 'finished\n' >> "$LOG"
