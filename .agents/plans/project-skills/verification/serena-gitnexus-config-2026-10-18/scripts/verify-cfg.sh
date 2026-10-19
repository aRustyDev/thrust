#!/bin/bash
# Verify the committed Serena + GitNexus layout against the REAL repository (this worktree).
SC=$SCRATCH
WT=$WORKTREE
LOG=$SC/verify-cfg.log
: > "$LOG"
note() { printf '%s\n' "$*" >> "$LOG"; }
cd "$WT" || exit 1

note "##### GITNEXUS (scratch GITNEXUS_HOME, so the real registry is untouched)"
export GITNEXUS_HOME=$SC/gn-home4
rm -rf "$GITNEXUS_HOME"; mkdir -p "$GITNEXUS_HOME"
git status --short > "$SC/cfg-status-before.txt"
note "-- status before analyze:"; cat "$SC/cfg-status-before.txt" >> "$LOG"
gitnexus analyze --name thrust-verify "$WT" >> "$LOG" 2>&1
note "[gn-analyze-plain] exit=$?"
git status --short > "$SC/cfg-status-after.txt"
note "-- status after analyze (must equal 'before'):"; cat "$SC/cfg-status-after.txt" >> "$LOG"
diff "$SC/cfg-status-before.txt" "$SC/cfg-status-after.txt" >> "$LOG" 2>&1
note "[status-diff] exit=$? (0 = identical)"
note "-- AGENTS.md / CLAUDE.md untouched:"; git diff --stat -- AGENTS.md CLAUDE.md >> "$LOG" 2>&1; note "(empty above = untouched)"
note "-- .claude/skills/gitnexus created?"; ls -A .claude 2>&1 | head -5 >> "$LOG"
note "-- a real crate symbol IS indexed (from_env in thrust-contract-tests):"
gitnexus context from_env -r thrust-verify 2>&1 | head -12 >> "$LOG"
note "-- a symbol that exists only under .agents/ must be ABSENT (proptest_config):"
gitnexus context proptest_config -r thrust-verify 2>&1 | head -6 >> "$LOG"
note "[gn-context-agents] done"

note "-- CONTROL: a copy WITHOUT .gitnexusignore, indexed with --skip-git: proptest_config must be FOUND"
CP=$SC/thrust-copy
rm -rf "$CP"; mkdir -p "$CP"
cp -R "$WT/." "$CP/" 2>> "$LOG"
rm -f "$CP/.git" "$CP/.gitnexusignore"; rm -rf "$CP/.gitnexus"
gitnexus analyze --skip-git --index-only --name thrust-ctrl "$CP" >> "$LOG" 2>&1
note "[gn-analyze-control] exit=$?"
gitnexus context proptest_config -r thrust-ctrl 2>&1 | head -8 >> "$LOG"
unset GITNEXUS_HOME

note "##### SERENA (a nested Claude starts the serena MCP server with --project-from-cwd in this worktree)"
note "-- before: symlink and target:"
ls -ld .github/config/serena >> "$LOG" 2>&1
ls -A .agents/config/serena >> "$LOG" 2>&1
claude -p "Reply with exactly: OK" --tools "" --no-session-persistence --output-format text 2>&1 | grep -v 'mcp-sdk' >> "$LOG"
note "[serena-claude] done"
note "-- after: symlink intact? real dir created beside it? what did Serena write into the target?"
ls -ld .github/config/serena >> "$LOG" 2>&1
ls -A .github/config >> "$LOG" 2>&1
ls -A .agents/config/serena >> "$LOG" 2>&1
note "-- git status (cache must not appear):"; git status --short >> "$LOG" 2>&1
note finished
