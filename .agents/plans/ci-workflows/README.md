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
| `research/0019-ci-baseline-after-migration.md` | what `ci` costs after the migration, measured the way 0017 was, and how much of the difference is the migration's (the five `verdict` jobs: 43 runner-seconds, 15 to 19 s on the critical path) |
| `research/0020-ci-baseline-after-the-test-phase.md` | what `ci` costs after the Test phase, against 0019: a full run's median wall clock 1024 s to 585 s (−43%), runner-seconds 1 to 4% lower (the two medians disagree), the e2e cell now alone on the critical path, and the one job that got slower (`pg-adapter`) without an attributed cause |
| `scripts/ci-commands.py`, `scripts/compare-job-sets.py`, `scripts/critical-path.py`, `scripts/cicd-table-vs-tree.py` | the migration's evidence tools: the step-by-step parity of two versions of the workflows; the job sets of two runs; where a run's wall clock goes; `cicd.md`'s file table against the tree |
| `research/0018-gate-spike.md` | how the one required `gate` judges a job inside a called workflow: five designs against seven scenarios, eleven facts measured, and the design chosen (A+); the evidence behind ADR-0102 |
| `phases/0001-foundation.md` | Phase 0 planned stage by stage: what exists today, the spike that chooses how `gate` judges a called workflow, the planner and gate changes, the lint conventions, the baseline, and the questions for the owner (answered) |
| `phases/0003-test.md` | Phase 2, Test, planned: what `test.yml` and `flake-hunt.yml` run today and what it costs, the interface and state machine, the acceptance criteria, the eight stages in order, the action candidates, and the four questions for the owner (answered) |
| `phases/0004-build.md` | Phase 3, Build, planned: what builds today and what the deploy bundle's 297 seconds are, why the bundle stays signed by `deploy.yml` and `release.yml`, the interface and state machine, the acceptance criteria, the six stages in a proposed order, the action candidates, and the four questions for the owner |
| `phases/0005-lint.md` | Phase 5, Lint: what `lint.yml` holds and what read the shell before, the facts that shape the phase (each measured), the interface, the acceptance criteria, and stage 1 in detail: `bash`, shellcheck over the maintained scripts and the composite actions' `run:` blocks (the owner's three answers, the survey, the custom code and why it has no built-in, the first run and what became of it, the probes, the pass log). The other stages are planned when they start |
| `phases/0002-migration.md` | Phase 1, the migration: what "the same" means and how each pull request proves it, what existed, the order of the stages (the simplest callee first), each of the six stages in detail with what it showed, the questions for the owner (answered), and the acceptance |

Phase documents (`phases/NNNN-*.md`) are written when a phase starts, not before. Phase numbers are allocated across the
whole repository: Phase 0 of this plan is `0001` and Phase 1 is `0002`; each later phase takes the next free number.

## Status (2026-11-22)

Phase 0, the foundation, is done and merged. Phase 1, the migration, is **done**: its six stages are merged (`documentation.yml`, #254; `security.yml`, #258;
`web.yml`, #264; `lint.yml`, #269; `test.yml`, split in two, #275 and #281; `e2e.yml`, #289), `ci.yml` holds only `changes`, the calls and `gate`, and its
acceptance is recorded in [phases/0002-migration.md](phases/0002-migration.md), with the cost after it in [research 0019](research/0019-ci-baseline-after-migration.md).
Phase 2, Test, is planned in [phases/0003-test.md](phases/0003-test.md) and its four questions are answered. All eight of its stages are done, and its acceptance is recorded there and in [research 0020](research/0020-ci-baseline-after-the-test-phase.md): all seven of its criteria hold, the last by the owner's decision of 2026-11-22 to leave the rest of G-5 by hand, so **Phase 2 is done**. Phase 3, Build, is planned in [phases/0004-build.md](phases/0004-build.md): its four questions are answered (all as recommended) and stage 1, `linux`, the template, is done (#352); stage 2, `wasm`, is done (#356, and the bundle now restores its cache); stage 3, `macos`, is next. Phase 5, Lint, is planned in [phases/0005-lint.md](phases/0005-lint.md), and taken up before Build is done because its first stage waits for nothing in it: stage 1, `bash` (shellcheck over the maintained scripts and the composite actions' `run:` blocks), is in #360. Phases 4 and 6 to 14 are not started; see the table at the end of `PLAN.md`.
