#!/bin/bash
# GitNexus: symlinked index dir, clean/re-analyze, .gitnexusrc (indexOnly) and .gitnexusignore, each with a control.
SC=$SCRATCH
LOG=$SC/cfg-test2.log
: > "$LOG"
G="git -c user.name=probe -c user.email=probe@example.invalid"
note() { printf '%s\n' "$*" >> "$LOG"; }
mkrepo() {
  rm -rf "$1"; mkdir -p "$1/src" "$1/ignored"
  cd "$1" || exit 1
  git init -q
  printf '[package]\nname = "probe"\nversion = "0.0.0"\nedition = "2021"\n' > Cargo.toml
  printf 'pub fn add(a: u32, b: u32) -> u32 { a + b }\npub fn use_add() -> u32 { add(1, 2) }\n' > src/lib.rs
  printf 'pub fn hidden_fn() -> u32 { 7 }\n' > ignored/secret.rs
}
export GITNEXUS_HOME=$SC/gn-home2
rm -rf "$GITNEXUS_HOME"; mkdir -p "$GITNEXUS_HOME"

note "##### A. symlinked index dir, a tracked file inside it, a pre-existing .gitignore in it"
mkrepo "$SC/gn2-a"
mkdir -p .agents/config/gitnexus
printf 'tracked readme\n' > .agents/config/gitnexus/README.md
printf '# mine\n!README.md\n' > .agents/config/gitnexus/.gitignore
ln -s .agents/config/gitnexus .gitnexus
$G add -A; $G commit -q -m init
gitnexus analyze --index-only --name gn2-a "$SC/gn2-a" >> "$LOG" 2>&1
note "[A-analyze] exit=$?"
note "-- .gitignore inside the dir after analyze (was '# mine / !README.md'):"; cat .agents/config/gitnexus/.gitignore >> "$LOG" 2>&1
note "-- git status after analyze (clean = nothing below):"; git status --short >> "$LOG" 2>&1
note "-- gitnexus clean:"
gitnexus clean --force >> "$LOG" 2>&1
note "[A-clean] exit=$?"
note "-- .gitnexus after clean:"; ls -ld .gitnexus >> "$LOG" 2>&1
note "-- dir after clean:"; ls -A .agents/config/gitnexus >> "$LOG" 2>&1
note "-- git status after clean:"; git status --short >> "$LOG" 2>&1
note "-- re-analyze after clean:"
gitnexus analyze --index-only --name gn2-a "$SC/gn2-a" >> "$LOG" 2>&1
note "[A-reanalyze] exit=$?"
ls -ld .gitnexus >> "$LOG" 2>&1
git status --short >> "$LOG" 2>&1

note "##### B. .gitnexusrc indexOnly: plain analyze must not inject AGENTS.md / CLAUDE.md / skills"
mkrepo "$SC/gn2-b"
printf '{ "indexOnly": true, "defaultBranch": "main" }\n' > .gitnexusrc
$G add -A; $G commit -q -m init
gitnexus analyze --name gn2-b "$SC/gn2-b" >> "$LOG" 2>&1
note "[B-analyze-with-rc] exit=$?"
note "-- files at root (with rc):"; ls -A >> "$LOG" 2>&1
note "-- CONTROL: same repo shape WITHOUT the rc, plain analyze (expect AGENTS.md/CLAUDE.md/.claude to appear):"
mkrepo "$SC/gn2-c"
$G add -A; $G commit -q -m init
gitnexus analyze --name gn2-c "$SC/gn2-c" >> "$LOG" 2>&1
note "[C-analyze-no-rc] exit=$?"
ls -A >> "$LOG" 2>&1

note "##### D. .gitnexusignore: ignored/ must not be indexed (control: gn2-c has no ignore file)"
mkrepo "$SC/gn2-d"
printf 'ignored/\n' > .gitnexusignore
$G add -A; $G commit -q -m init
gitnexus analyze --index-only --name gn2-d "$SC/gn2-d" >> "$LOG" 2>&1
note "[D-analyze] exit=$?"
note "-- context hidden_fn WITH ignore (expect not found):"
gitnexus context hidden_fn >> "$LOG" 2>&1
note "[D-context-with-ignore] exit=$?"
cd "$SC/gn2-c" || exit 1
note "-- context hidden_fn WITHOUT ignore (control, expect found):"
gitnexus context hidden_fn >> "$LOG" 2>&1
note "[C-context-control] exit=$?"
note finished
