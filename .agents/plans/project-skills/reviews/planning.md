# Review: planning (round 1)

Reviewer: skill-reviewer (fresh). Read all 7 files and compared the draft with the convention it was adapted from. Nothing executed.

**Verdict: PASS (Section A clean). Grade: B.** Not ready to adopt until one sanitization leak was fixed (done after this review).

## Section A
- `allowed-tools`: `Edit(.agents/plans/**)` and `Edit(docs/src/**)` only; no Bash, no wildcard; `Edit` rules cover all file-editing tools.
- The injection-shaped string in eval 5 is quoted test data (not addressed to a reviewer); finding 5 asked for it to be labelled.
- No network, credential or system-config access.

## Sanitization sweep (requirement: no project-internal or organisation-specific names, paths or jargon in the shipped skill)
Result at review time: **NOT CLEAN, one leak:** an example identifier in `references/plan-layout.md` that only made sense in the setting the convention was adapted from. Minor inferences: `SOURCES.md` advertised that names had been removed and cited an unverifiable source. Everything else environment-specific had been removed (organisation, repository and directory names, tracker terms, hostnames, home paths, the worked example).

## Findings
1. HIGH sanitization: replace the leaked example identifier with a generic qualified spec number.
2. MEDIUM accuracy: the MADR claim was partly false: the minimal template has no Confirmation; the full template keeps status in YAML front matter (`proposed | rejected | accepted | deprecated | superseded by ADR-NNNN`) and nests Confirmation under Decision Outcome.
3. MEDIUM trigger precision: bare "create a plan" / "where does this go" collide with implementation-plan requests; add a NOT-for line and near-miss negatives.
4. MEDIUM functional gap with `mdbook`: pages created or moved under `docs/src/` must be linked from `docs/src/SUMMARY.md`, else they are silently omitted.
5. MEDIUM evals: label eval 5 as an adversarial fixture and add a discriminating expectation; widen expectation 3.
6. LOW `allowed-tools` semantics: path rules resolve relative to the working directory; moving or deleting needs a shell and prompts.
7. LOW internal consistency: four-digit numbers vs a two-digit example; numbering undefined for analysis/findings/PRDs; "more binding" never ordered.
8. LOW scope contradiction: the description excluded AGENTS.md but the body governed root `AGENTS.md` / `CLAUDE.md`.
9. LOW duplication of facts across SKILL.md and references.
10. LOW GitNexus note: its licence (PolyForm Noncommercial 1.0.0) was verified and rules out bundling it; remove or sharpen the pointer (its output goes to a third home).
11. LOW eval coverage: number allocation, spec qualification.

## Collisions
`gitnexus-plan`, `writing-plans`, `context-placement` overlap "create a plan" / "where does this go" (tightened by finding 3); `codex-handoff` consumes `.claude/plans/` (informational); `mdbook` complementary.

## Follow-up applied after this review
Findings 1-11 were addressed in the shipped text; the GitNexus pointer was removed from `SKILL.md` and kept only in `references/SOURCES.md`. The revised text was not re-reviewed by `skill-reviewer`; it was checked with a mechanical frontmatter/links/leak check and an independent search for project-internal and organisation-specific terms.
