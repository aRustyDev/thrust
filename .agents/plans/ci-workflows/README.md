# ci-workflows

Splits CI, which is one 26 KB workflow (`ci.yml`) plus seven others, into one workflow per concern (Test, Lint, Docs,
Security, Web, E2E, Build, Release, Deploy, PR, Chores, Bench, Prof, Perf, UI/UX, Issue), with the steps they repeat
factored into actions. Done looks like this: every concern has its own workflow file; `ci.yml` is only the planner, the
calls and the one required `gate`; and every workflow has been through the eight passes (proof of concept to
production-grade documentation) in the owner's priority order.

The work runs in three steps. **Phase 0** makes the split safe (the orchestrator pattern, a way for the gate to
judge called workflows that a skipped job cannot fool, the planner and the workflow lint). **Phase 1** moves today's jobs into the new files as a pure
refactor: same commands, same results, no passes. **Phases 2 to 14** then take each workflow through all eight passes.

How CI works today is not repeated here: [CI/CD](../../../docs/src/architecture/cicd.md) is the reference, and
[ADR-0100](../contrail/adrs/ci/0100-build-matrix-per-adapter-set.md) and
[ADR-0101](../contrail/adrs/ci/0101-ci-supply-chain.md) are the decisions behind it. The review that produced the
current release flow is in [cicd-review](../cicd-review/README.md).

## Map

| Path | Contents |
|---|---|
| `PLAN.md` | the owner's decisions; the architecture; Phase 0 and the migration; the priority-ordered phase table; the templates for a phase and a stage; what each pass must show; the invariants every pull request keeps; the status table |
| `research/0017-ci-baseline.md`, `scripts/ci-baseline.py` | what `ci` costs today (per job and per run, from GitHub's own records), and the script that measures it again; the number the passes that save time and the migration are checked against |
| `research/0018-gate-spike.md` | how the one required `gate` judges a job inside a called workflow: five designs against seven scenarios, eleven facts measured, and the design chosen (A+); the evidence behind ADR-0102 |
| `phases/0001-foundation.md` | Phase 0 planned stage by stage: what exists today, the spike that chooses how `gate` judges a called workflow, the planner and gate changes, the lint conventions, the baseline, and the questions for the owner (answered) |
| `phases/0002-migration.md` | Phase 1, the migration, planned: what "the same" means and how each pull request proves it, what exists today, the order of the stages (the simplest callee first), the first stage in detail, and the questions for the owner |

Phase documents (`phases/NNNN-*.md`) are written when a phase starts, not before. Phase numbers are allocated across the
whole repository: Phase 0 of this plan is `0001` and Phase 1 is `0002`; each later phase takes the next free number.

## Status (2026-11-17)

Phase 0, the foundation, is done and merged. Phase 1, the migration, is planned, its four questions are answered, its first
stage, `documentation.yml`, is merged (#254), its second, `security.yml`, is merged (#258), its third, `web.yml`, is merged (#264), and its fourth, `lint.yml`, is in progress; see
the table at the end of `PLAN.md`.
