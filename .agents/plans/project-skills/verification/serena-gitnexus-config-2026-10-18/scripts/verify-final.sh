#!/bin/bash
# FINAL verification of the committed layout: .gitnexusrc (indexOnly only) and the normalized Serena project.yml.
SC=$SCRATCH
WT=$WORKTREE
LOG=$SC/verify-final.log
: > "$LOG"
note() { printf '%s\n' "$*" >> "$LOG"; }
cd "$WT" || exit 1
rm -rf .gitnexus

note "##### config files as committed"
note "-- rc:"; cat .agents/config/gitnexus/gitnexusrc.json >> "$LOG"
ls -l .gitnexusrc .github/config/serena >> "$LOG" 2>&1
ls .gitnexusignore >> "$LOG" 2>&1; note "(no .gitnexusignore expected above)"

note "##### GITNEXUS plain analyze, scratch GITNEXUS_HOME"
export GITNEXUS_HOME=$SC/gn-home5
rm -rf "$GITNEXUS_HOME"; mkdir -p "$GITNEXUS_HOME"
git status --short > "$SC/fin-before.txt"
gitnexus analyze --name thrust-final "$WT" > "$SC/fin-analyze.txt" 2>&1
note "[gn-analyze] exit=$?"
grep -c -i 'default branch' "$SC/fin-analyze.txt" >> "$LOG"; note "(count of 'default branch' warnings above, expect 0)"
grep -E 'indexed successfully|nodes' "$SC/fin-analyze.txt" >> "$LOG"
git status --short > "$SC/fin-after.txt"
diff "$SC/fin-before.txt" "$SC/fin-after.txt" >> "$LOG" 2>&1
note "[status-diff] exit=$? (0 = identical)"
ls CLAUDE.md >> "$LOG" 2>&1; note "(no CLAUDE.md expected above: analyze must not create one)"
git diff --stat -- AGENTS.md >> "$LOG" 2>&1; note "(AGENTS.md diff above, expect only my own edit)"
gitnexus context from_env -r thrust-final 2>&1 | head -6 >> "$LOG"
unset GITNEXUS_HOME
rm -rf .gitnexus

note "##### SERENA second start leaves project.yml byte-identical and does not re-save"
shasum .agents/config/serena/project.yml >> "$LOG"
claude -p "Reply with exactly: OK" --tools "" --no-session-persistence --output-format text 2>&1 | grep -v 'mcp-sdk' >> "$LOG"
shasum .agents/config/serena/project.yml >> "$LOG"
note "(two identical hashes above)"
LATEST=$(ls -t "$HOME/.serena/logs/$(date +%Y-%m-%d)" | head -1)
printf 'resave lines in the newest Serena log: ' >> "$LOG"
grep -c -E 'incomplete|re-saving' "$HOME/.serena/logs/$(date +%Y-%m-%d)/$LATEST" >> "$LOG"
grep -h -E "Activating|Language server startup \(language=rust\) starting" "$HOME/.serena/logs/$(date +%Y-%m-%d)/$LATEST" | cut -c1-200 | head -3 >> "$LOG"
ls -ld .github/config/serena >> "$LOG" 2>&1
git status --short >> "$LOG" 2>&1
note finished
