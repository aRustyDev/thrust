# Plan directory layout and numbering

A plan is a directory under `.agents/plans/`. This is the **full** set of things a plan may contain; most plans have a handful. Create each entry on its first real document.

```text
.agents/plans/<plan-name>/
├── assets/          INPUTS: img/ audio/ video/ pdf/ js/ css/ html/
├── artifacts/       OUTPUTS: agent-produced, meant to be read by a human
├── scripts/         *.js *.ts *.sh *.py
├── phases/          nnnn-*.md
├── questions/       nnnn-*.md
├── research/        nnnn-*.md
├── analysis/        nnnn-*.md
├── findings/        nnnn-*.md
├── reviews/         verdicts on drafts produced by the plan
├── specs/           nnnn-*.md
├── prds/            nnnn-*.md
├── justfile         the plan's own entrypoints
├── README.md        what this plan is; the map to everything below
├── PLAN.md          ROADMAP.md   PRD.md    SPEC.md
└── ANALYSIS.md      FINDINGS.md  QUESTIONS.md  RESEARCH.md
```

## Singular and plural

The singular files (`PLAN.md`, `SPEC.md`, `FINDINGS.md`, ...) are the plan-level document; the matching plural directory holds its numbered constituents. Use one, both, or neither, whichever the plan actually has. A plan-specific `PLAN.md` or `ROADMAP.md` stays in the plan: only cross-plan roadmaps graduate to `docs/src/dev/roadmaps/`.

## Numbering

Always four digits (`0001`). To allocate, list the existing files of that kind and take the highest number plus one.

| Kind | Scope of the sequence | How to refer to it |
|---|---|---|
| ADR, question, research, phase | the whole repository | by number (`ADR-0012`, `Q-0005`) |
| spec, PRD, analysis, finding | per product or plan | always qualified: `billing/SPEC-0003`, never `SPEC 3` |

A plan whose questions start at `0005` is normal: the sequence is global, so gaps appear under other plans. Say so in the plan's README rather than renumbering.

## A status line is not a stub

A plan that is mostly decided but not yet shipped records its state at the top of `PLAN.md` (a short dated status table with open items). That is a document with content, not a scaffold.
