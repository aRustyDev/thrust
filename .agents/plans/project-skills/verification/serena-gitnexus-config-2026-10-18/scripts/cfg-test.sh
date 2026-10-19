#!/bin/bash
# Can GitNexus and Serena keep their per-repo directories under .agents/config/** through a symlink?
# Isolated: scratch repos, and GITNEXUS_HOME points at a scratch directory so the real registry is untouched.
SC=$SCRATCH
SRC=$WORKTREE_A/.github/config/serena/project.yml
LOG=$SC/cfg-test.log
: > "$LOG"
G="git -c user.name=probe -c user.email=probe@example.invalid"
mkrepo() {
  mkdir -p "$1/src"
  cd "$1" || exit 1
  git init -q
  printf '[package]\nname = "probe"\nversion = "0.0.0"\nedition = "2021"\n' > Cargo.toml
  printf 'pub fn add(a: u32, b: u32) -> u32 { a + b }\n' > src/lib.rs
  $G add -A
  $G commit -q -m init
}
note() { printf '%s\n' "$*" >> "$LOG"; }

note "##### GITNEXUS: .gitnexus as a symlink into .agents/config/gitnexus"
GN=$SC/gn-link-test
rm -rf "$GN"; mkdir -p "$GN/home"
mkrepo "$GN/repo"
mkdir -p .agents/config/gitnexus
ln -s .agents/config/gitnexus .gitnexus
note "-- symlink:"; ls -ld .gitnexus >> "$LOG" 2>&1
export GITNEXUS_HOME="$GN/home"
gitnexus analyze --index-only --name link-probe "$GN/repo" >> "$LOG" 2>&1
note "[gn-analyze] exit=$?"
note "-- is .gitnexus still a symlink?"; ls -ld .gitnexus >> "$LOG" 2>&1
note "-- files in .agents/config/gitnexus:"; ls -la .agents/config/gitnexus >> "$LOG" 2>&1
note "-- git status (what would be committed):"; git status --short >> "$LOG" 2>&1
gitnexus status >> "$LOG" 2>&1
note "[gn-status] exit=$?"
unset GITNEXUS_HOME

note "##### SERENA: .github/config/serena as a symlink into .agents/config/serena"
SR=$SC/serena-link-test
rm -rf "$SR"
mkrepo "$SR"
mkdir -p .agents/config/serena .github/config
sed -e 's/^project_name:.*/project_name: "link-probe-xyz"/' "$SRC" > .agents/config/serena/project.yml
printf '/cache\n/project.local.yml\n' > .agents/config/serena/.gitignore
ln -s ../../.agents/config/serena .github/config/serena
note "-- symlink:"; ls -ld .github/config/serena >> "$LOG" 2>&1
note "-- project_name in the target:"; grep -n '^project_name' .github/config/serena/project.yml >> "$LOG" 2>&1
claude -p "Call the serena get_current_config tool (use whichever serena server is available) and reply with exactly one line: ACTIVE_PROJECT=<the active project name>. If no project is active or the tool is unavailable, reply: NONE" \
  --allowedTools "mcp__serena__get_current_config,mcp__plugin_serena_serena__get_current_config" \
  --tools "" --no-session-persistence --output-format text 2>&1 | grep -v 'mcp-sdk' >> "$LOG"
note "[serena-claude] done"
note "-- after: is the symlink intact and what did Serena write?"
ls -ld .github/config/serena >> "$LOG" 2>&1
ls -la .agents/config/serena >> "$LOG" 2>&1
note "-- real dir created beside it?"; ls -la .github/config >> "$LOG" 2>&1
note "finished"
