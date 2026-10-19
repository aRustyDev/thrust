#!/bin/bash
# Does Claude Code load AGENTS.md natively? Three scratch directories, unique markers, no tools.
SC=$SCRATCH
T=$SC/cm-test
LOG=$SC/cm-test.log
: > "$LOG"
rm -rf "$T"
mkdir -p "$T/a-agents-only" "$T/b-import" "$T/c-claude-no-import"
printf '# Rules\n\nMARKER-ALPHA: the secret handshake word is pelican.\n' > "$T/a-agents-only/AGENTS.md"
printf '# Rules\n\nMARKER-ALPHA: the secret handshake word is pelican.\n' > "$T/b-import/AGENTS.md"
printf '@AGENTS.md\n' > "$T/b-import/CLAUDE.md"
printf '# Rules\n\nMARKER-ALPHA: the secret handshake word is pelican.\n' > "$T/c-claude-no-import/AGENTS.md"
printf '# Local\n\nMARKER-BETA: the other word is walrus.\n' > "$T/c-claude-no-import/CLAUDE.md"

PROMPT='Using ONLY the project instructions already loaded in your context (no tools, do not guess): what is the secret handshake word, and what is the other word? Reply in exactly this format and nothing else: handshake=<word or NONE>; other=<word or NONE>'
for d in a-agents-only b-import c-claude-no-import; do
  cd "$T/$d" || exit 1
  printf '== %s\n' "$d" >> "$LOG"
  ls >> "$LOG" 2>&1
  claude -p "$PROMPT" --tools "" --no-session-persistence --output-format text 2>&1 | grep -v 'mcp-sdk' >> "$LOG"
  printf '[%s] done\n' "$d" >> "$LOG"
done
printf 'finished\n' >> "$LOG"
