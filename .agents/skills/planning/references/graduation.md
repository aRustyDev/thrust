# Specs, PRDs and graduation

## Specs and PRDs live in two trees; the tree declares which kind

Same format, two different jobs. Do not assume a `SPEC.md` is a target just because it is a spec.

| Tree | Kind | What it states | An unmet criterion is... |
|---|---|---|---|
| `docs/src/dev/{specs,prds}/` | **graduated** | the base, core, current state, expectations and requirements of the project, in the present tense | a factual report about reality, not an outstanding task: the task lives in the issue tracker |
| `.agents/plans/<plan>/{specs,prds}/` | **aspirational** | objectives and goals for long-running work and narrower scopes: what *should* become true | an open objective of the plan |

## Graduating a plan document

A plan document **graduates** when its content stops being a target and becomes a standing property of the project.

1. Move it from `.agents/plans/<plan>/...` to the matching `docs/src/dev/...` home (table in `SKILL.md`). Moving needs a shell, which prompts.
2. **Rewrite** it into the present tense. A graduated document says what is true, not what will be.
3. Leave no copy behind. Replace the plan-side file with a one-line link, or delete it if nothing links to it; fix every inbound reference, including the `docs/src/SUMMARY.md` entry for the new page.

A move without the rewrite is the failure mode: the docs then claim a target as fact.

## Rules by document kind

| Document | Rule |
|---|---|
| ADR | written in `docs/` and never graduated; **immutable once accepted**: to change a decision, write a new ADR that supersedes it and mark the old one superseded |
| strategy | evolves by dated amendment (append, do not rewrite history) |
| policy | names its gate command; executable where possible |
| pattern | advisory; never states a MUST |
