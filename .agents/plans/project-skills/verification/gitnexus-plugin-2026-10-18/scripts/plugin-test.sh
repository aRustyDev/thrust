#!/bin/bash
# Does Claude Code read the project's .claude/settings.json plugin keys? Isolated config dir, with a control.
SC=$SCRATCH
WT=$WORKTREE
LOG=$SC/plugin-test.log
: > "$LOG"
note() { printf '%s\n' "$*" >> "$LOG"; }
rm -rf "$SC/cc-empty" "$SC/plugin-control"
mkdir -p "$SC/cc-empty" "$SC/plugin-control"

note "##### 1. isolated EMPTY config dir, inside the repo WITH the project settings"
cd "$WT" || exit 1
CLAUDE_CONFIG_DIR="$SC/cc-empty" claude plugin list --json > "$SC/pl-repo.json" 2> "$SC/pl-repo.err"
note "[list-in-repo] exit=$?"
note "-- raw output (first 600 bytes):"; head -c 600 "$SC/pl-repo.json" >> "$LOG"; note ""
note "-- stderr (first 400 bytes):"; head -c 400 "$SC/pl-repo.err" >> "$LOG"; note ""

note "##### 2. CONTROL: same isolated config dir, in a directory WITHOUT project settings"
cd "$SC/plugin-control" || exit 1
CLAUDE_CONFIG_DIR="$SC/cc-empty" claude plugin list --json > "$SC/pl-ctrl.json" 2> "$SC/pl-ctrl.err"
note "[list-control] exit=$?"
head -c 600 "$SC/pl-ctrl.json" >> "$LOG"; note ""
head -c 400 "$SC/pl-ctrl.err" >> "$LOG"; note ""

note "##### 3. real user config, in the repo: the gitnexus entry only"
cd "$WT" || exit 1
claude plugin list --json 2>/dev/null | jq -c '[.[]? | select((.id // .name // "") | test("gitnexus")) | {id, version, scope, enabled, projectPath}]' >> "$LOG" 2>&1
note finished
