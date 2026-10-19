#!/bin/bash
# Does Claude Code's own debug output show it resolving the project's plugin settings? Isolated config dir + control.
SC=$SCRATCH
WT=$WORKTREE
LOG=$SC/plugin-test2.log
: > "$LOG"
note() { printf '%s\n' "$*" >> "$LOG"; }
rm -rf "$SC/cc-empty2"; mkdir -p "$SC/cc-empty2" "$SC/plugin-control"
for where in "$WT" "$SC/plugin-control"; do
  cd "$where" || exit 1
  note "##### in: $where"
  CLAUDE_CONFIG_DIR="$SC/cc-empty2" claude -p "Reply with exactly: OK" --tools "" --no-session-persistence --output-format text --debug-file "$SC/dbg.txt" < /dev/null > "$SC/dbg-out.txt" 2>&1
  note "[run] exit=$?"
  head -c 200 "$SC/dbg-out.txt" >> "$LOG"; note ""
  note "-- debug lines mentioning plugins / marketplaces / gitnexus:"
  grep -i -E 'gitnexus|marketplace|enabledPlugins|plugin' "$SC/dbg.txt" 2>/dev/null | cut -c1-240 | head -14 >> "$LOG"
  note "-- count of such lines: $(grep -c -i -E 'gitnexus|marketplace|enabledPlugins|plugin' "$SC/dbg.txt" 2>/dev/null)"
  rm -f "$SC/dbg.txt"
done
note finished
