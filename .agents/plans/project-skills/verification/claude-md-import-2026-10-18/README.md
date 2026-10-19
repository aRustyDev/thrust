# Verification pack: how Claude Code loads AGENTS.md and CLAUDE.md (2026-10-18)

Evidence for the root `CLAUDE.md` (`@AGENTS.md`). Claude Code 2.1.285, print mode with `--tools ""` (the model cannot read
files, so an answer can only come from instructions already loaded into its context), `--no-session-persistence`.

| Log | What it shows |
|---|---|
| `logs/cm-test.log` (`scripts/cm-test.sh`) | Three scratch directories with unique marker words. `a-agents-only` (AGENTS.md alone): the marker was known, so AGENTS.md is loaded without a CLAUDE.md. `b-import` (CLAUDE.md = `@AGENTS.md`): the marker was known. `c-claude-no-import` (a CLAUDE.md that does not import AGENTS.md): the AGENTS.md marker was **not** known (`handshake=NONE`) and only the CLAUDE.md marker was. So AGENTS.md is a fallback used only when there is no CLAUDE.md |
| `logs/real-repo-before-claude-md.txt` | This repository, AGENTS.md present and no CLAUDE.md: the model quoted rule 2 verbatim. The control "AGENTS.md is not loaded without a CLAUDE.md" did **not** fail |
| `logs/real-repo-after-claude-md.txt` | This repository with `CLAUDE.md` (the import plus an HTML comment): the model still quoted rule 2 |

Consequence: today the import changes nothing, because AGENTS.md already loads as the fallback. It matters the moment
anyone adds a `CLAUDE.md`: without the import line, every rule in AGENTS.md silently stops loading (scratch run `c`).
An earlier statement in PR #16 that Claude Code does not load AGENTS.md without a CLAUDE.md was wrong for this version.

Not run: other Claude Code versions or hosts, and the effect of the HTML comment on the loaded text (the import works
with it present; whether the comment itself reaches the model was not tested).
