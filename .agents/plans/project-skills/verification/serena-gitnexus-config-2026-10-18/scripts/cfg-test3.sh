#!/bin/bash
# GitNexus: config files kept in .agents/config/gitnexus behind ROOT symlinks; index stays in the default .gitnexus/.
SC=$SCRATCH
LOG=$SC/cfg-test3.log
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
export GITNEXUS_HOME=$SC/gn-home3
rm -rf "$GITNEXUS_HOME"; mkdir -p "$GITNEXUS_HOME"

note "##### E. config in .agents/config/gitnexus, root symlinks, plain analyze (no flags)"
mkrepo "$SC/gn3-e"
mkdir -p .agents/config/gitnexus
printf '{ "indexOnly": true, "defaultBranch": "main" }\n' > .agents/config/gitnexus/gitnexusrc.json
printf '.agents/\nignored/\n' > .agents/config/gitnexus/gitnexusignore
ln -s .agents/config/gitnexus/gitnexusrc.json .gitnexusrc
ln -s .agents/config/gitnexus/gitnexusignore .gitnexusignore
note "-- root symlinks:"; ls -l .gitnexusrc .gitnexusignore >> "$LOG" 2>&1
$G add -A; $G commit -q -m init
gitnexus analyze --name gn3-e "$SC/gn3-e" >> "$LOG" 2>&1
note "[E-analyze] exit=$?"
note "-- root listing (no AGENTS.md / CLAUDE.md / .claude expected):"; ls -A >> "$LOG" 2>&1
note "-- git status (index must not show up):"; git status --short >> "$LOG" 2>&1
note "-- add is indexed:"
gitnexus context add -r gn3-e >> "$LOG" 2>&1
note "[E-context-add] exit=$?"
note "-- hidden_fn (in ignored/) must be absent:"
gitnexus context hidden_fn -r gn3-e >> "$LOG" 2>&1
note "[E-context-hidden] exit=$?"

note "##### F. control: the same files as REAL root files but with NO ignore file: hidden_fn must be found"
mkrepo "$SC/gn3-f"
printf '{ "indexOnly": true }\n' > .gitnexusrc
$G add -A; $G commit -q -m init
gitnexus analyze --name gn3-f "$SC/gn3-f" >> "$LOG" 2>&1
note "[F-analyze] exit=$?"
gitnexus context hidden_fn -r gn3-f >> "$LOG" 2>&1
note "[F-context-hidden] exit=$?"

note "##### G. clean leaves the config alone"
cd "$SC/gn3-e" || exit 1
gitnexus clean --force >> "$LOG" 2>&1
note "[G-clean] exit=$?"
ls -A >> "$LOG" 2>&1
ls -l .gitnexusrc .gitnexusignore >> "$LOG" 2>&1
git status --short >> "$LOG" 2>&1
gitnexus analyze --name gn3-e "$SC/gn3-e" >> "$LOG" 2>&1
note "[G-reanalyze] exit=$?"
git status --short >> "$LOG" 2>&1
note finished
