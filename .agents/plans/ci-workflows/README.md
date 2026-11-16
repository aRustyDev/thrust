# ci-workflows

Splits CI, which is one 26 KB workflow (`ci.yml`) plus six others, into one workflow per concern (Test, Lint, Docs,
Security, Web, E2E, Build, Release, Deploy, PR, Chores, Bench, Prof, Perf, UI/UX, Issue), with the steps they repeat
factored into actions. Done looks like this: every concern has its own workflow file; `ci.yml` is only the planner, the
calls and the one required `gate`; and every workflow has been through the eight passes (proof of concept to
production-grade documentation) in the owner's priority order.

The work runs in three steps. **Phase 0** makes the split safe (the orchestrator pattern, a nested gate that cannot be
fooled by a skipped job, the planner and the workflow lint). **Phase 1** moves today's jobs into the new files as a pure
refactor: same commands, same results, no passes. **Phases 2 to 14** then take each workflow through all eight passes.

How CI works today is not repeated here: [CI/CD](../../../docs/src/architecture/cicd.md) is the reference, and
[ADR-0100](../contrail/adrs/ci/0100-build-matrix-per-adapter-set.md) and
[ADR-0101](../contrail/adrs/ci/0101-ci-supply-chain.md) are the decisions behind it. The review that produced the
current release flow is in [cicd-review](../cicd-review/README.md).

## Map

| Path | Contents |
|---|---|
| `PLAN.md` | the owner's decisions; the architecture; Phase 0 and the migration; the priority-ordered phase table; the templates for a phase and a stage; what each pass must show; the invariants every pull request keeps; the status table |

Phase documents (`phases/NNNN-*.md`) are written when a phase starts, not before. Phase numbers are allocated across the
whole repository, and none exists yet, so the first is `0001`. ADR-0102 (the orchestrator pattern) is written in
Phase 0.

## Status (2026-11-16)

See the table at the end of `PLAN.md`. The plan is approved; nothing is built.
