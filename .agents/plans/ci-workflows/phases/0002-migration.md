# Phase 0002: Migration

Part of [ci-workflows](../PLAN.md). Planned 2026-11-17; M1 to M5b are built and merged (#254, #258, #264, #269, #275, #281) and M6 is in progress. Facts are from
`main` at `f263ab4`, read on that day: recount them when a stage starts. Phase 1 of the plan, after [Phase 0](0001-foundation.md) (the foundation: merged). It moves
today's jobs out of `ci.yml` into one workflow per concern as a **pure refactor**: the same commands, the same job boundaries,
the same result of `gate`. No pass applies to it; each stage's evidence is its acceptance criteria. The passes start in Phase 2,
on the migrated layout.

The plan's decisions are in [PLAN.md](../PLAN.md); the design that makes a called workflow judgeable is
[ADR-0102](../../contrail/adrs/ci/0102-one-workflow-per-concern-and-the-verdict-job.md), and what it rests on is
[research 0018](../research/0018-gate-spike.md). This document does not restate them.

## What "the same" means, and how each pull request proves it

A refactor is only a refactor if something shows it. Each stage's pull request carries:

1. **A parity table**: every step of every moved job, old place to new place, with its command byte for byte.
2. **A mechanical check of that table** (below), so that it is not only my reading.
3. **A full run and an affected-only run**, both green, running the same commands as before (a dispatch of `ci` is a full
   run: it has no base to diff).
4. **Two probes that make the real wiring fail**, as draft pull requests closed unmerged (like #196 to #199 and the spike):
   a planned job in the new workflow skipped by a bug in its `if`, and the new workflow's `verdict` job skipped. Each must turn
   the real `gate` red. This is invariant 3 of the plan, shown on the real files and not on a fixture.
5. The other invariants of the plan: `gate` stays the only required context, an `rc/*` pull request still plans a staging
   dispatch, the lint and the planner's tests pass, and every script of `deps` is run before the push.

**The mechanical check** is a script, `.agents/plans/ci-workflows/scripts/ci-commands.py`: it lists every step of every job of
every workflow (the job's id, the step's `uses:` and `with:` or its `run:` text, its `if:` and `env:`), and compares the list on
`main` with the list on the branch, treating a job as the same job wherever it lives. A step that disappeared, appeared or changed
is reported; a step that only moved is not. Its control is the reason to trust it: a copy of the branch with one step dropped,
and one with one command changed, must each be reported. *(Hand-written: nothing public compares the steps of two sets of
workflows; `actionlint` and `diff` on the YAML do not know that a job moved. The survey is repeated when the stage starts.)*

M3 added a rule for a workflow that bears its job's name (`web`): the id stays in `ci.yml` and holds the call, so the same id is in
both files. The script pairs them by what they are: a job that became a call (`uses:`, no steps) must find, under the same id in
another file, a job with the same fingerprint, and then it is `moved`; if it finds none its steps are `LOST`, however expected the
call is. Its self-test went from 24 to 33 checks, and four bugs planted in the new rule were each caught (the twin found by id
alone, the loss not reported, every changed job looked at, the twin left among the appearances).

## What exists today (read 2026-11-17)

| Fact | Where | Consequence |
|---|---|---|
| `ci.yml` has `changes`, ten jobs and `gate`; each job has `if: contains(fromJSON(needs.changes.outputs.jobs ...), '<id>')`. `changes` outputs `jobs`, `matrix`, `cells` only | `ci.yml` | A stage adds the outputs `top` and `plans` (the planner already prints them, Phase 0) and replaces the jobs it moves with one calling job |
| `gate` downloads the `cell-ok-*` pass markers itself and runs `assert-gate.sh` with `EXPECTED_JOBS=jobs`, `PLANNED=cells`, over its default job list | `ci.yml` `gate` | A moved job leaves `gate`'s list and its calling workflow joins it. The marker rule stays at the top until `contract` moves |
| The gate's default `KNOWN_JOBS` is a constant in `assert-gate.sh`, and `same_jobs` in `test-ci-scripts.sh` holds it equal to the planner's `JOBS` | `assert-gate.sh`, `test-ci-scripts.sh` | After a stage the constant is the planner's top-level names (the jobs still in `ci.yml`, then the workflows): `same_jobs` must compare that, so it is part of every stage |
| `VERDICT_JOBS` names the called workflows, and a name in it must be a job the gate knows | `assert-gate.sh` | `ci.yml` sets it as a **static list**, not from the planner's `workflows` output: a planner bug that dropped a workflow from that output would otherwise switch off the check for it. A test holds the list equal to the keys of `WORKFLOWS` |
| `flake-hunt.yml` sets a workflow-level `concurrency:` and reads an empty `inputs` as "a scheduled run" | `flake-hunt.yml` | Folding it into `test.yml` breaks R8 (no workflow-level `concurrency:` in a called workflow) and sits on the edge of R9 (a callee routes by an input): see the questions |
| `e2e-schedule.yml` calls `contract-cell.yml` twice, has a workflow-level `concurrency:`, and runs on a pull request that edits it | `e2e-schedule.yml` | A rename to `e2e.yml`, not a callee: it has its own triggers and `gate` does not wait for it |
| A called workflow's jobs are `<calling job> / <job>` in the check list, and a matrix call is `<calling job> (<cell>) / <job>` | research 0018, F4 | Nothing requires a name but `gate`. `ci-baseline.py` classifies a job by what precedes ` / `, so its `--groups` changes with the layout, as research 0017 says |
| Renaming a job id (`docs`, `ci-scripts`) touches about 20 and 40 lines of the planner, the gate, the tests and the mutants, and about 20 lines of documentation, each | counted 2026-11-17 | A rename is mechanical, but it is not free: see the questions |

## The stages, and the order

The plan listed M1 to M6 in the order of the owner's priority groups, with `test.yml` first. This phase takes **the simplest
callee first**, because the first real called workflow is where whatever the spike could not show will show, and that is
cheapest where there is least to move.

| Stage | Workflow | Moves | Why here |
|---|---|---|---|
| M1 | `documentation.yml` | `docs` | One job. No matrix, no cell markers, no secrets, no Docker. It exercises every part of the wiring: `changes`' outputs, the calling job, the `verdict` job, `gate`'s `VERDICT_JOBS`, the planner's map, the index, the probes |
| M2 | `security.yml` | `deny` | The second callee, so the `verdict` job's steps are now **the same in two places**: the composite action is made here, by the plan's rule (see below) |
| M3 | `web.yml` | `web` | One job again, with its own environment (Node, pnpm, and two environment variables at the job level) |
| M4 | `lint.yml` | `lint`, `deps` | Two jobs, and `deps` has eighteen steps (recounted when M4 started; `lint` has seven), the workflow guard and the two added in Phase 0 (the index guard, `actionlint`) among them: the first callee that holds more than one job, and the one that runs the guards of the files it is moving into |
| M5a | `test.yml` | `ci-scripts`, `hermetic`, `pg-adapter`, `mutants` | Four plain moves, like M4. `contract` and the cell-marker rule stay in `ci.yml` and `gate` for this stage (owner, 2026-11-17: M5 is split) |
| M5b | `test.yml` | `contract` | The matrix call (three levels deep), the cell markers, and the `verdict` job that must judge them: the `verdict` action gains `cell-job` and `cells`, and `gate` stops downloading markers. Done (#281) |
| M6 | `e2e.yml` | `e2e-schedule.yml` renamed | Not a callee: the name in the index, the docs and the planner test's list changes (the planner needs no rule, see M6 in detail) |

Every stage edits `ci.yml`, so they run one after another, each starting from the `main` that holds the last.

### M1 in detail: `documentation.yml`

**Purpose.** Move the book's check out of `ci.yml`, and prove the pattern of ADR-0102 on the smallest real job.

**Acceptance.**
- `.github/workflows/documentation.yml` (`name: documentation`, the lowercase of the repository's other workflows): triggers
  `workflow_call` (input `plan`, a JSON array of the jobs to run, required; output `verdict`) and `workflow_dispatch` (input `plan`,
  default `["docs"]`, so that a run by hand runs the workflow). No workflow-level `concurrency:`, `permissions: {}`, each job its
  own. It holds the job `docs`, **verbatim** (the same steps, the same pins, the same comment), with
  `if: contains(fromJSON(inputs.plan), 'docs')`, and the `verdict` job of ADR-0102, written out, not yet an action.
- `ci.yml`: `docs` is gone; a calling job `documentation` (`needs: changes`, `if` on `top`, `with: plan:` from `plans`);
  `changes` outputs `top` and `plans`; `gate` needs `documentation` and sets `EXPECTED_JOBS` to `top`, `VERDICT_JOBS` to
  `documentation` and the default list to the new names.
- `plan-ci.py`: `WORKFLOWS = {"documentation": ("docs",)}`. `assert-gate.sh`'s default list, `same_jobs`, the index row, the
  `cicd.md` text, `ci-baseline.py`'s `--groups` and the mutant driver follow. The mutant driver is run: every mutant is still
  killed and none is broken, and a mutant of the new wiring (the gate without `VERDICT_JOBS`) is added.
- The book is built by the same commands: `scripts/check-book.sh` and `scripts/check-book.sh --self-test`, in that order.
- Proof, as above: the parity table and its check, a full run and an affected-only run (a change under `docs/` plans the
  `documentation` workflow and nothing else; a change to `deny.toml` plans no `documentation`), and the two probes.

**State machine of the job.** `docs` is planned, then runs, then succeeds (the workflow's `verdict` job judges it) or fails (the
`verdict` fails, `documentation` fails, `gate` fails); `docs` not planned is skipped, and its `verdict` expects that. The
`verdict` job is `if: always()`, so it reports when `docs` is skipped, failed or cancelled.

**Steps.** The workflow file and the `ci.yml` edit first; then the planner's map and the gate's list with their tests (red, then
green); then the index, the docs and the parity script; then the probes. Details of the job's steps are not repeated: they are
moved, not written.

**Public solutions.** None applies to a move. The `verdict` job's judge is `assert-gate.sh`, which research 0018 measured
against `re-actors/alls-green` and kept. The composite action for the `verdict` job is **not made in M1**, by the plan's rule:
with one callee its steps are not repeated anywhere, and an action drawn from one example guesses its inputs.

### M1: done (#254, merged 2026-11-17 as `8905e4c`)

What it showed, as the stage's own acceptance asked:
- **Parity**, by `scripts/ci-commands.py`: 34 jobs before, 36 after; 30 unchanged, `docs` moved with its 4 steps identical, 3
  changed as intended (`changes`, `gate`, `ci-scripts`), 2 appeared (`ci.yml:documentation`, `documentation.yml:verdict`), 0 gone.
- **The wiring**, by `test_workflow_wiring.py` (35 checks) and `mutate_gate.py` (76 mutants killed, the 4 that change nothing
  survived); a full run of 40 checks, `gate` certified, `documentation / docs` built the book.
- **Two probes** that made the real `gate` red, each for its own reason: #255 (`docs` planned and skipped) failed
  `documentation / verdict` and `gate`; #256 (the step that sets `verdict=pass` skipped) failed `gate` alone, with `documentation
  succeeded but its verdict output is 'missing', not 'pass'`.

What it found: the gate walks `needs` in the order `ci.yml` lists them, so a probe that also reddens `ci-scripts` hides its own
message (plant a fault the text tests cannot see); `actionlint` refuses a constant `if: false`; a skip of the `verdict` job itself
cannot be planted alone, because the wiring test checks `if: always()`, so a probe skips the step that sets the output instead.

**A correction to M1's pull request**, which said the book and `deny.toml` cases would be "the next PR's (M2) first real
affected-only run". They will not: every stage edits `ci.yml`, which the planner runs in full. The affected-only run is taken once,
after M6, as the migration's acceptance; until then those cases are `test_plan_ci.py`'s.

### M2 in detail: `security.yml`, and the `verdict` action

**Purpose.** Move the supply-chain check out of `ci.yml`, and, because the `verdict` job's steps now stand in two files, make them
a composite action, by the plan's rule: an exact duplicate in two places is made into an action without asking, and the pull
request says so.

**The duplicate, side by side.** The `verdict` job of `documentation.yml` (M1) and the one `security.yml` needs differ in one
value, the jobs the workflow knows (`docs`, `deny`); every other line is the same: the runner, the timeout, the permission, the
checkout, the judge step with its five variables, and the step that sets the output.

| | In the action | Stays in the job |
|---|---|---|
| Judge step: `assert-gate.sh` with `NEEDS_JSON`, `EXPECTED_JOBS`, `KNOWN_JOBS`, and `CHANGES_JOB`, `CELL_JOB` both set empty | yes | |
| The step that sets the output `verdict=pass`, after the judge | yes | |
| `actions/checkout` | | **yes**: a local action is read from the checkout, so the checkout comes first and an action cannot make it |
| `runs-on`, `timeout-minutes`, `permissions`, `needs`, `if: always()`, the job's `outputs` | | yes: they belong to the job (`.github/actions/README.md` says so) |

Inputs of the action: `needs` (`toJSON(needs)` of the calling job), `plan` (the workflow's `plan` input) and `jobs` (the jobs the
workflow knows); output: `verdict`. A variable the two copies share and the action fixes: `CHANGES_JOB` and `CELL_JOB` empty, since a
called workflow has neither a planner job nor cells. `test.yml`'s `verdict` job (M5) does have cells, so it is **not** an exact copy and is
asked about when M5 starts, with the two side by side; nothing here guesses its inputs.

**Acceptance.**
- `.github/actions/verdict/action.yml` (a row in `.github/actions/README.md`), and `documentation.yml`'s `verdict` job calls it: the
  same job, with the judge and mark steps replaced by one step.
- `.github/workflows/security.yml` (`name: security`): `workflow_call` (input `plan`, required; output `verdict`) and
  `workflow_dispatch` (`plan` defaults to `["deny"]`); no workflow-level `concurrency:`; `permissions: {}`; the job `deny` moved
  **verbatim** (steps, pins, comment) with `if: contains(fromJSON(inputs.plan), 'deny')` and without `needs: changes`; a `verdict`
  job that calls the action.
- `ci.yml`: `deny` is gone; a calling job `security`; `gate` needs it, and `VERDICT_JOBS` is `documentation security`.
- The planner: `WORKFLOWS` holds `security: (deny)`; a rule makes a change to the action run everything (the gate trusts it, like
  `assert-gate.sh`); the gate's default list, `same_jobs`, the indexes, `cicd.md`, `running.md`, `ci-baseline.py`'s groups and the mutant
  driver follow. A change to `deny.toml` plans the workflow `security` and nothing else; a change to a crate's `Cargo.toml` plans it
  too (cases of `test_plan_ci.py`).
- The wiring test checks the action once (its judge, its two fixed variables, its output set after the judge and only there) and
  each called workflow's `verdict` job (the call and its three inputs). The mutant driver gains mutants of the action and of
  `security.yml`; every mutant is killed, none is broken, the equivalent ones survive.
- Proof as for M1: the parity table and its check (`deny` moved, its steps identical; the `verdict` job of `documentation.yml` the one
  change to an existing file's job), a full run, and two probes planted in `security.yml`: `deny` planned and skipped, and the call
  of the action skipped. The second also shows that `documentation`, which shares the action, still passes.

**State machine of the job.** As `docs`: planned, then runs, then succeeds or fails; not planned, then skipped. The `verdict` job
is `if: always()`.

**Public solutions.** None applies to the move. The judge is `assert-gate.sh` (research 0018 measured `re-actors/alls-green` against
it). The job runs `cargo deny` as `scripts/rs cargo deny --locked check`, from `taiki-e/install-action`. Whether a public
cargo-deny action fits better is a question for the Security phase's survey (pass 1, not made here); a refactor does not swap it,
since that changes the command.

### M2: done (#258, merged 2026-11-17 as `1bc9e9a`)

What it showed:
- **Parity**, by `scripts/ci-commands.py`: 36 jobs before, 39 after; 33 unchanged, `deny` moved with its 4 steps identical, 2 changed
  as intended (`gate`: its `VERDICT_JOBS`; `documentation.yml`'s `verdict`: its two steps became one call), 3 appeared
  (`ci.yml:security`, `security.yml:verdict`, the action), 0 gone.
- **The wiring**: `test_workflow_wiring.py` 63 checks (the action once, each workflow's call), the gate's cases 182, the planner's 6,983;
  `mutate_gate.py` 115 mutants killed, 6 that change nothing survived, on the runner too; a full run of 41 checks, `gate` certified.
- **Two probes**: #259 (`deny` planned and skipped) failed `security / verdict` and `gate`, `documentation` still passed; #260 (the
  call of the action skipped) failed `gate` alone, `security succeeded but its verdict output is 'missing', not 'pass'`.

What it found: a local action is read from the checkout, so the checkout stays in each `verdict` job (the first plan said the action
would hold it); and a run of the mutant driver is void if a file under `.github/` changes while it runs, since it copies the tree for
each mutant.

### M3 in detail: `web.yml`

**Purpose.** Move the stock app's build check out of `ci.yml`, the third callee, which calls the `verdict` action as the first two do.

**What is particular to it.** The workflow and its job share a name, `web`. In `ci.yml` the calling job keeps the id `web` (so `gate`'s
`needs` list does not change), and inside `web.yml` the job is `web` too: the checks are `web / web` and `web / verdict`. The planner's
`top` lists the jobs still in `ci.yml` and then the workflows, so `web` moves to the end of the gate's default list
(`... mutants documentation security web`); the planner already tests a workflow that shares its job's name (`pc.slices({"web", ...},
{"web": ("web",)})`), and what it does not yet do is the wiring test, which says that a moved job is no longer a job of `ci.yml` and
must now allow the calling job of the same name, and say that no id is used twice.

**Acceptance.**
- `.github/workflows/web.yml` (`name: web`): `workflow_call` (input `plan`, required; output `verdict`) and `workflow_dispatch` (`plan`
  defaults to `["web"]`); no workflow-level `concurrency:`; `permissions: {}`; the job `web` moved **verbatim** (the job-level
  `TURBO_TELEMETRY_DISABLED` and `DO_NOT_TRACK`, the checkout, the Q-R2 gate, Node, the pnpm pin, the frozen install, the build, the
  check that it made the SPA; the pins and the comment) with `if: contains(fromJSON(inputs.plan), 'web')` and without `needs: changes`;
  and a `verdict` job that calls the action.
- `ci.yml`: the `web` job is a call of `web.yml` (`needs: changes`, runs when `top` names `web`, `plan` from `plans`); `gate` needs it
  as before, and `VERDICT_JOBS` is `documentation security web`.
- The planner: `WORKFLOWS` holds `web: (web)`; the gate's default list, `same_jobs`, the indexes, `cicd.md`, `running.md` and the
  mutant driver (a row in `CALLS`) follow. A change under `web/` plans the workflow `web` and, as before, both browser journey cells.
- The wiring test allows a moved job's name in `ci.yml` only as the calling job of that name, and refuses an id used twice; the mutant of
  "the moved job is left in `ci.yml`" becomes, for `web`, a second job of the same name.
- Proof as for M2: the parity table and its check (`web` moved, its 7 steps identical), a full run, and two probes planted in
  `web.yml`: `web` planned and skipped, and the call of the action skipped.

**State machine of the job.** As `docs` and `deny`.

**Public solutions.** None applies to the move. Whether a public action for pnpm fits better than `npm install --global pnpm@<the pin
in web/package.json>` is a question for the Web phase's survey, not for a refactor, which keeps the commands.

### M3: done (#264, merged 2026-11-17 as `af9f365`)

What it showed:
- **Parity**, by `scripts/ci-commands.py`: 39 jobs before, 41 after; 37 unchanged, `web` moved with its 7 steps identical, 2 changed as
  intended (the `web` job of `ci.yml`, now the call; `gate`: its `VERDICT_JOBS`), 1 appeared (`web.yml:verdict`), 0 gone.
- **The wiring**: `test_workflow_wiring.py` 82 checks, the gate's cases 186, the planner's 6,986; `mutate_gate.py` 136 mutants killed, the 7
  that change nothing survived, on the runner too; a full run of 42 checks, `gate` certified.
- **Two probes**: #265 (`web` planned and skipped) failed `web / verdict` and `gate`, with `documentation` and `security` still green; #266
  (the call of the action skipped) failed `gate` alone, `web succeeded but its verdict output is 'missing', not 'pass'`.

What it found: a workflow that bears its job's name (`web` calls `web.yml`, which holds the job `web`) has the id in both files, which
two tools had to learn (the wiring test allows a moved job's name in `ci.yml` only as the call that bears it and refuses an id used
twice; the parity script now says `LOST` for a job that became a call and whose steps are nowhere else); and `main` moved three times
while the stage was built, so the build was committed, `main` merged in, and everything run again on the merged tree. The owner's
deferral of `build-web` is in the decisions at the end.

### M4 in detail: `lint.yml`

**Purpose.** Move the Rust lint and the repository's guards out of `ci.yml`: the fourth callee, and the first that holds two jobs.

**What is particular to it.**
1. **Two jobs, so a plan can be partial.** A change to `clippy.toml` plans `lint` and not `deps`; a change to a guard of `deps` plans `deps`
   (and `ci-scripts`) and not `lint`. The `verdict` job judges both: a planned job must be `success`, and one that is not planned must be
   `skipped`. The gate-level cases for it exist (a callee with two planned jobs and one unplanned); this stage adds the planner's real
   cases through the command line, and the probe skips one of the two.
2. **The name clash again.** The workflow `lint` bears the name of its job `lint`, as `web` does, and `deps` is a plain move. The checks
   are `lint / lint`, `lint / deps` and `lint / verdict`. The calling job in `ci.yml` keeps the id `lint`; `deps` leaves `gate`'s `needs`.
3. **`deps` runs the guards of the files it moves into.** In `lint.yml` it lints `lint.yml` itself, and every other workflow and action:
   the workflow lint (R1 to R9), the index guard and `actionlint`. So the run of this very pull request is what shows `deps` green in its new
   home; no probe is needed for that, and none could replace it.
4. **`deps` keeps `fetch-tags: true` on its checkout** (the release guard compares the workspace with the last tag) and its job
   `timeout-minutes: 20`; `lint` its 20 as well. `deps` is 18 steps (the checkout, `setup-rust`, and sixteen guards or checks, ending in the
   OpenAPI diff, which needs Docker) and `lint` is 7.
5. **The mutant driver's `CALLS` rows hold one job each**; this stage lets a row hold several, and makes the mutants of a workflow with
   several jobs (a verdict job that waits for one only, one that knows one only, a job that runs whatever the plan, per job).

**Acceptance.**
- `.github/workflows/lint.yml` (`name: lint`): `workflow_call` (input `plan`, required; output `verdict`) and `workflow_dispatch` (`plan`
  defaults to `["deps","lint"]`); no workflow-level `concurrency:`; `permissions: {}`; the jobs `deps` and `lint` moved **verbatim** (steps,
  pins, comments, `fetch-tags`) with `if: contains(fromJSON(inputs.plan), '<id>')` and without `needs: changes`; and a `verdict` job that
  needs both and calls the action.
- `ci.yml`: the `deps` job is gone; the `lint` job is a call of `lint.yml` (`needs: changes`, runs when `top` names `lint`, `plan` from `plans`);
  `gate` needs `lint` as before and no longer `deps`; `VERDICT_JOBS` is `documentation security web lint`.
- The planner: `WORKFLOWS` holds `lint: (deps, lint)`; the gate's default list becomes `ci-scripts hermetic pg-adapter contract mutants
  documentation security web lint`; `same_jobs`, the indexes, `cicd.md`, `running.md`, `ci-baseline.py`'s groups (`deps` leaves the
  default list: `lint / deps` is in the group `lint`) and the mutant driver follow. A change to `clippy.toml` plans the workflow `lint` with
  the plan `["lint"]`, and to `scripts/check-layout.sh` the plan `["deps"]` (cases of `test_plan_ci.py`).
- The wiring test needs no new rule (it already holds a list of jobs and a call that bears its job's name); the mutant driver gains the
  mutants of a two-job workflow.
- Proof as for M3: the parity table and its check (`deps` moved with its 18 steps and `lint` with its 7, every one identical), a full run
  (the `deps` job green in `lint.yml` is the proof of 3), and two probes planted in `lint.yml`: `deps` planned and skipped (with `lint`
  still passing, so the verdict names one job), and the call of the action skipped.

**State machine of each job.** As `docs`, `deny` and `web`.

**Public solutions.** None applies to the move. The guards are the repository's own scripts; `actionlint` is installed from its release
archive with a pinned SHA-256 because `taiki-e/install-action` does not offer it (Phase 0). Whether `clippy` and `rustfmt` fit a public
lint action is a question for the Lint phase's survey, not for a refactor, which keeps the commands.

**No action is proposed.** `lint` and `deps` share a checkout and a `setup-rust` call and nothing else, and `setup-rust` already is the
action; the `deps` guards are pairs of a check and its `--self-test`, which a loop in a script would hide, not an action.

### M4: done (#269, merged 2026-11-17 as `099d6d7`)

What it showed:
- **Parity**, by `scripts/ci-commands.py`: 41 jobs before, 43 after; 38 unchanged, `deps` moved with its 18 steps and `lint` with its 7,
  every one identical, 2 changed as intended (the `lint` job of `ci.yml`, now the call; `gate`), 1 appeared (`lint.yml:verdict`), 0 gone.
  The cut of `ci.yml` was done by a script that refused unless every line it removed was in `lint.yml`, in order.
- **The wiring**: `test_workflow_wiring.py` 100 checks, the gate's cases 182, the planner's 6,993; `mutate_gate.py` 165 mutants killed, the 8
  that change nothing survived, on the runner too; a full run of 43 checks, `gate` certified. The OpenAPI diff was run locally for the first time.
- **Two probes**: #270 (`deps` planned and skipped, `lint` untouched) failed `lint / verdict` and `gate`, the verdict naming the one of the two
  jobs that did not run while `lint / lint` stayed green; #271 (the call of the action skipped) failed `gate` alone.

What it found: `deps` ran its guards inside `lint.yml`, on `lint.yml` itself, and was green, which is the proof that the job works in its new
home; the mutant driver's `CALLS` rows needed to hold a list of jobs; and earlier pull-request bodies had said "every guard of `deps`" ran
locally when the OpenAPI diff (Docker) had not.

### M5a in detail: `test.yml`, without `contract`

**Purpose.** Move the four plain jobs of the Test concern out of `ci.yml`, as in M4, and leave the one that is different (`contract`, its
matrix call and the cell-marker rule) for M5b, where it is tested alone (owner's decision, 2026-11-17, below).

**What is particular to it.**
1. **`contract` and the marker rule stay.** `ci.yml` keeps the job `contract` and `gate` keeps downloading the `cell-ok-*` markers and judging
   the cells. So `top` lists `contract` and then the workflows, and the gate's default list is `contract documentation security web lint test`.
2. **No name clash.** No job is called `test`, so the calling job `test` is new and `gate` needs it in place of four.
3. **The four jobs are in three stretches of `ci.yml`**, with the `lint` call and `contract` between them; the cut script of M4 takes the
   three stretches and checks each removed line against `test.yml`.
4. **`ci-scripts` tests the files it moves into**: it runs the wiring test and the mutant driver, which read `test.yml`. Its own run is the proof.
5. **`hermetic` uploads an artifact** (`junit-hermetic`); a name that does not match `gate`'s `cell-ok-*` pattern, so the marker rule is not
   disturbed. The jobs differ in timeout (10, 30, 30 and 45 minutes) and in what they set up, and are moved verbatim.
6. **A workflow of four jobs**: the plan may name any of them (a deploy script plans `ci-scripts` alone; a change to a crate plans
   `ci-scripts`, `hermetic` and more).

**Acceptance.**
- `.github/workflows/test.yml` (`name: test`): `workflow_call` (input `plan`, required; output `verdict`) and `workflow_dispatch` (`plan`
  defaults to the four jobs); no workflow-level `concurrency:`; `permissions: {}`; the four jobs moved **verbatim** in the order they have,
  with `if: contains(fromJSON(inputs.plan), '<id>')` and no `needs: changes`; and a `verdict` job that needs the four and calls the action.
- `ci.yml`: the four jobs are gone; a calling job `test` (`needs: changes`, runs when `top` names `test`, `plan` from `plans`); `gate` needs
  `test` in place of the four, and `VERDICT_JOBS` is `documentation security web lint test`.
- The planner: `WORKFLOWS` holds `test: (ci-scripts, hermetic, pg-adapter, mutants)`; the gate's default list, `same_jobs`, the indexes,
  `cicd.md`, `running.md`, `ci-baseline.py`'s groups and the mutant driver (a row in `CALLS`) follow. A change to a deploy script plans the
  workflow `test` with the plan `["ci-scripts"]`.
- Proof as for M4: the parity table and its check (the four jobs moved, every step identical), a full run, and two probes planted in
  `test.yml`: one of the four planned and skipped (with the others still passing), and the call of the action skipped.

**State machine of each job.** As the others.

**Public solutions.** None applies to the move. The four jobs run the repository's own scripts and `cargo nextest`, installed by
`setup-rust`; a public action for any of them is a question for the Test phase's survey (Phase 2), not for a refactor.

**No action is proposed.** The four jobs share a checkout and a `setup-rust` call and nothing else.

### M5a: done (#275, merged 2026-11-17 as `bb24076`)

What it showed:
- **Parity**, by `scripts/ci-commands.py`: 43 jobs before, 45 after; 38 unchanged, `ci-scripts`, `hermetic`, `pg-adapter` and `mutants` moved with
  their 2, 7, 4 and 3 steps, every one identical, 1 changed as intended (`gate`), 2 appeared (`ci.yml:test`, `test.yml:verdict`), 0 gone.
  `test.yml` and the cut of `ci.yml` were generated from `ci.yml`'s own text, and the script refused unless all 149 removed lines were found in order.
- **The wiring**: `test_workflow_wiring.py` 118 checks, the gate's cases 162, the planner's 6,998 on the runner; `mutate_gate.py` 199 mutants
  killed, the 9 that change nothing survived, on the runner too; a full run of 44 checks, `gate` certified. `test / ci-scripts` ran its whole
  list, the wiring test and the mutant driver included, in `test.yml`.
- **Two probes**: #276 (`hermetic` planned and skipped, the other three jobs green) failed `test / verdict` and `gate`, the verdict naming
  `hermetic`; #277 (the call of the action skipped) failed `gate` alone.

What it found: a **semantic merge conflict**: while `ci-scripts` was being moved, #272 added a test to its command list in `ci.yml`. Git saw a
deletion against an addition; the resolution kept the deletion and put the line in `test.yml`, checked by a script (main's other 51 lines were
already there) and by the parity check against `main` as it then stood. Three mutants went BROKEN, because the move had changed their anchor
text, and were re-anchored (the gate's list no longer starts with `ci-scripts`; `hermetic`'s `if` is gone from `ci.yml`; `hermetic`'s JUnit
upload step has an `if: always()` of its own).

And one thing that was not M5a's: `main`'s run on its merge commit was **red**, in `security / deny`, from a new RustSec advisory
(RUSTSEC-2026-0543, `chrono` unmaintained, no safe upgrade), the kind of failure that job's comment says to expect with no change to the code.
#279 added a documented temporary exception to `deny.toml`.

### M5b in detail: `contract`, and the judging of its cells

**Purpose.** Move the last job of the Test concern, `contract`, and with it the rule that judges its cells, out of `ci.yml` and `gate`. After
it `ci.yml` holds `changes`, the calls and `gate`, which is the migration's acceptance for `ci.yml`.

**What is particular to it.**
1. **The matrix call is nested.** `ci.yml` calls `test.yml` (the job `test`), which calls `contract-cell.yml` once per cell (the job `contract`,
   a matrix): three levels, against a limit of ten. The planner's `matrix` and `cells` outputs reach `test.yml` as two new inputs, strings that
   hold JSON (`matrix`, default `{"include":[]}`, and `cells`, default `[]`). `contract-cell.yml` itself, and its other callers
   (`e2e-schedule.yml`, and `flake-hunt.yml` through an action), are not touched.
2. **The cells are judged by `test / verdict`**, as you decided: the `verdict` action gains the optional inputs `cell-job` (default empty:
   none, as for the four other callees) and `cells` (default `[]`). When `cell-job` is set, a first step downloads the `cell-ok-*` markers, as
   `gate`'s step does today (`continue-on-error`: with no marker the judge fails on "no cell markers"), and the judge is given `CELL_JOB`,
   `PLANNED` and `REPORTED_DIR`. The markers are artifacts of the same run, so the download works across the nested call; the run is what proves it.
3. **`gate` stops judging cells.** Its marker step and its `PLANNED` and `REPORTED_DIR` go. `assert-gate.sh`'s defaults describe the top-level
   gate, which now has no cells: the default cell job becomes none (it was `contract`) and the default known jobs are the five called workflows.
   **The cost, said plainly:** a verdict job that forgot `CELL_JOB` used to fail, because the default asked for cells it was not given
   (`test-ci-scripts.sh` has that case). With the default none, a callee that forgets it passes without judging cells. The property moves to
   the places that can hold it: the wiring test requires `test.yml`'s call of the action to pass `cell-job: contract` and `cells`, and a
   mutant (the call forgets `cell-job`) shows the test can fail.
4. **The gate's cell cases move to the callee level, they are not dropped.** The cells' cases (a planned cell that never reports, a marker for
   an unplanned cell, no cell planned, no marker, cells planned and the cell job not expected, the unset `PLANNED` and `REPORTED_DIR`) and the
   per-job loop's special case for `contract` are re-expressed with `CELL_JOB=contract` at the level of a verdict job.
5. **`top` is only workflows now**: no job is left in `ci.yml` for the gate to judge directly.
6. **The check names change** (a cell becomes a call two levels down); `ci-baseline.py` classifies cells by name, so its handling is read from
   the first run and adapted.

**Acceptance.**
- `test.yml`: inputs `plan`, `matrix` and `cells` (`workflow_dispatch` has them with defaults); the job `contract` moved from `ci.yml` as a
  matrix call of `contract-cell.yml` (`if` on the plan, the matrix from the input, the same `with:`, `fail-fast: false`, `permissions: contents: read`), placed
  between `pg-adapter` and `mutants` as in `ci.yml`; the `verdict` job needs the five jobs and passes `cell-job: contract` and `cells`.
- `.github/actions/verdict/action.yml`: the two inputs, the conditional download, the three variables from the inputs.
- `ci.yml`: `contract` is gone; the `test` call passes `matrix` and `cells` from `changes`; `gate` needs `[changes, test, lint, web, documentation,
  security]`, downloads nothing, and passes no `PLANNED` or `REPORTED_DIR`.
- The planner's map holds `test: (ci-scripts, hermetic, pg-adapter, contract, mutants)`; the gate's defaults; the tests (the planner's, the gate's
  at both levels, the wiring test for the cell call and the action's new inputs); the mutant driver (a row of five jobs, and the mutants that
  only cells have: the call without `cell-job` or `cells` or `matrix`, a gate that still downloads markers, the action that ignores `cell-job`,
  never downloads, or reads another directory than it downloaded to); the indexes, `cicd.md`, `running.md`, `ci-baseline.py`.
- Proof: parity (`contract` moved with its call identical), a full run (**the 26 cells reported to `test / verdict`**, and `gate` still
  certified), and two probes: **a planned cell whose marker is never uploaded** (planted in `contract-cell.yml`: the cell succeeds, and
  `test / verdict` must say `planned cells that never reported success`), and the call of the action skipped.

**State machine of the job.** `contract` is planned, then runs one job per planned cell and uploads a marker for each cell that passed; it
succeeds when every cell does. `test / verdict` then judges the five jobs and the cells: every planned cell has a marker and no marker is
for an unplanned cell.

**Public solutions.** None applies to the move. The judge is `assert-gate.sh`; the download is `actions/download-artifact`, which the gate
already uses at the same pin.

**Questions for the owner:** none. The two that this stage had (the shape of the action, the split) are answered above.

### M5b: done (#281, merged 2026-11-17 as `5cadf2f`)

What it showed:
- **Parity**, by `scripts/ci-commands.py`: 53 jobs before and after; 48 unchanged and 5 changed as intended (the `verdict` action, `ci.yml:test`,
  `ci.yml:gate`, `test.yml:verdict`, `test.yml:contract`), none appeared or gone. `contract` is the same call (its `fail-fast`, `with` and
  `permissions` included) and differs in one line: the matrix comes from `inputs.matrix`, not from `needs.changes.outputs.matrix`. It was cut
  from `ci.yml` by a script that refused unless every removed line but the three that differ by design was found in `test.yml`, in order.
- **The wiring**: `test_workflow_wiring.py` 136 checks, the gate's cases 196 (162 before: the cell cases were re-run at the level of a verdict job,
  not dropped), the planner's 7,008; `mutate_gate.py` 226 mutants killed, the 9 that change nothing survive, on the runner too. I read the first
  failing check of each of the 27 new mutants: each fails on its own fault.
- **Two full runs**, one on the build (`f5f2ca7`) and one on the head after merging `main` (`4c3c833`): 42 of 42 jobs, `gate` certified. **The markers
  crossed the nested call**: `test / verdict` collected the `cell-ok-*` artifacts and said `26 of 26 planned cells reported success`, three levels
  down from `ci.yml`; `gate` judged no cell.
- **Two probes**: #282 (a planned cell succeeded and never uploaded its marker) failed `test / verdict`, with `planned cells that never reported
  success: native-memory sweeper`, and `gate`, with nothing else red; #283 (the call of the action skipped) failed `gate` alone, with every job
  and every cell green, which is the case that matters most here, because with that call skipped no cell is judged at all.
- **`main`'s own run on the merge commit** (a push run): 42 of 42 jobs succeeded, the first run of the nested cells on `main`.

What it found:
- **The cost of the design, as planned**: a verdict job that forgets `CELL_JOB` no longer fails by default. The wiring test and its mutants carry the
  property now. A parallel session built the same stage as #285 and was closed on the owner's word; its one extra rule, that `assert-gate.sh`
  refuses a non-empty `PLANNED` when no cell job is named, would make a forgotten `cell-job` fail whenever cells are planned. It was not taken
  into #281: it is a change to the gate's behaviour, and an offer to the owner.
- **A cell's check name changed** (`test / contract (<target>, <pass>) / <target> <pass>`), so `ci-baseline.py`'s split on the first caller name
  would have counted the 26 cells as `test`'s. It recognises a cell by its own shape now (`CELL`), with a control; its self-test went from 38 to 44.
- **A mistake of mine, and its fix**: `git commit -a` swept a local `Cargo.lock` reorder into the first commit of probe #282 (main's lock had a
  package out of cargo's order after #279; #284 fixed it on `main`). A follow-up commit restored `main`'s lock, so the probe's net diff was its one line.
  Stage by explicit path in a worktree where cargo runs.

### M6 in detail: `e2e.yml`

**Purpose.** Name the daily browser-journey workflow after its concern: `e2e-schedule.yml` becomes `e2e.yml` (`name: e2e`), as the plan's layout
says. It is the last stage of the migration; the migration's acceptance follows it.

**What is particular to it.**
1. **It is not a callee.** It has triggers of its own (a schedule, a dispatch, and a pull request that edits the file), `gate` does not read it,
   and it has no `verdict` job, no entry in `VERDICT_JOBS` and none in the planner's map. None of the callee stages' wiring applies to it, and
   none of their proofs can show anything about it.
2. **What changes**: the file name, `name:` (`e2e-schedule` to `e2e`), the `paths:` filter of its own pull-request trigger, and the
   concurrency group (`e2e-schedule-${{ github.ref }}` to `e2e-${{ github.ref }}`; a top-level workflow may have one, R8 binds only callees).
   The job ids `e2e` and `e2e-mail`, their calls of `contract-cell.yml`, their `with:` and their `permissions:` do not change. Four comments that
   M5b made false (they say `ci.yml` runs the cell and that `ci.yml`'s `contract` job grants `contents: read`; both are `test.yml`'s now) are
   corrected in the same edit. A comment is not a command and `ci-commands.py` does not compare comments, so that diff is read by eye.
3. **Where the name is used.** The planner test's `NO_CHECK_WORKFLOWS` (`e2e-schedule` to `e2e`); the workflows index; `cicd.md` (the table
   row and two sentences); `testing/techniques/app-and-e2e.md`; `contrail/TRACEABILITY.md`. The **planner itself needs no change**: `e2e.yml` is
   a file that runs no check, so it falls under the generic `.github/workflows/` rule exactly as the old file did (a change to it plans the
   helper tests and the workflow lint). The dated records of runs under the old name (`testing/status.md` G-31, `cicd-review/FINDINGS.md`) stay as
   written, and `cicd.md`'s row says the file was named so.
4. **The run history splits.** Runs of the old file stay under `e2e-schedule`; the new file starts its own, and the schedule moves with the
   file: the first scheduled run under `e2e` is the next 04:41 UTC after the merge.
5. **Its own pull request runs it.** The `paths:` filter now names `e2e.yml`, so the pull request that adds the file runs the two cells: the proof
   that the renamed call is valid (a reusable workflow called with the wrong permissions fails at startup, with no job).
6. **The first affected-only run of the migration.** M6 does not edit `ci.yml`, so for the first time a stage's own run is affected-only (the
   files it changes plan `lint`, `test` and `documentation`, not `web` or `security`). That is one real data point for the acceptance, not the
   whole of it: the acceptance chooses changes that exercise each callee.

**Acceptance.**
- `e2e.yml` exists and `e2e-schedule.yml` does not, in one rename (`git mv`, so the history follows); the two calls are identical.
- `rg e2e-schedule` finds only the dated records and the plan documents that tell this history.
- The planner test lists `e2e` and refuses a workflow file that is in no list; the index guard passes with the new row.
- Proof: parity (`e2e` and `e2e-mail` moved with identical calls), the pull request's own run (affected-only, green) **and the e2e workflow's own
  run on it (both cells green)**, and two probes.

**The probes are not the callee stages' probes, and why.** `gate` does not read `e2e`, so a planned job skipped or a skipped `verdict` cannot
turn it red and would show nothing. The faults a rename can leave behind that the gate **does** see are two stale references, so a probe plants
each in the finished build, one per draft pull request, closed unmerged:
- **A: the planner test's list still names `e2e-schedule`.** `e2e.yml` is then in no list, and `test / ci-scripts` must fail on it (and `gate`).
- **B: the index still has the row for `e2e-schedule.yml` and none for `e2e.yml`.** `check-ci-index.sh`, in `lint / deps`, must fail on it (and `gate`).
  *Corrected after the run:* **two** guards refuse it, not one: the guard's own test (`test-check-ci-index.sh`, which runs the guard on the real
  repository) fails in `test / ci-scripts` too, as the planner's rule says (a change to an index runs the guard and its test). The plan
  named only the first because the probe was checked locally against the guard and not against its test.
Each is first checked locally to fail for its own reason alone (for B, with the guard's test as well).

**What it will not show.** That the schedule fires under the new file name (it is read from the run list the morning after the merge); and that
a failing `e2e` blocks nothing, which is unchanged and not probed.

**State machine.** Unchanged: a schedule, a dispatch or an edit of the file starts it; the two cells run in parallel through `contract-cell.yml`;
a failure notifies and blocks nothing.

**Public solutions.** None applies to a rename.

**Questions for the owner:** none. The rename is the plan's own decision.

## Decisions on the questions (owner, 2026-11-17, after #251)

All four were answered as recommended.

| Question | Answer |
|---|---|
| The order | The simplest callee first: M1 `documentation.yml`, `test.yml` fifth. Your priority groups govern the passes of Phases 2 to 14; nothing in the migration depends on them, and `test.yml` is the stage most likely to find something the spike did not |
| The job ids | Kept through the migration (`docs`, `ci-scripts`, `lint`, `deny`, ...), and renamed in the phase that owns the workflow. A rename is about 60 lines of the planner and its tests and 20 to 40 of documentation per job, and `ci-scripts` is not one-to-one with `bash` (it also runs the planner's Python tests and the mutants) |
| `flake-hunt.yml` | Kept exactly as it is through the migration, and a stage of the Test phase (Phase 2), where `test.yml` is being reworked anyway. Folding it in needs a job-level `concurrency:` (R8 forbids the workflow-level one it has) and reads "no `mode` given" as "the weekly hunt" (R9), a refactor that bends two of its own rules for nothing the Test phase does not buy at less risk |
| `name:` of a workflow | Lowercase, the repository's convention (`documentation`, `security`, `test`): the name shows in every check and in `workflow_run`'s `workflows:` list |

No question is open.

## Decisions made during the stages (owner)

| After | Question | Answer |
|---|---|---|
| #258 (2026-11-17) | A shared `build-web` action for the Node, pnpm, install, build and SPA steps of the `web` job and of `contract-pass-tools`'s e2e passes. They are the same commands, but not an exact copy: four steps with job-level env against one step with step-level env, and `contract-pass-tools` runs two more commands after | **Deferred.** M3 moves `web` as it is. The action is proposed again in the Web and E2E phases, where pass 5 (caching the pnpm store) touches exactly these steps |
| #269 (2026-11-17) | How `test.yml`'s `verdict` job gets the cell judging (the same judge and mark steps as the `verdict` action, plus a step that downloads the `cell-ok-*` markers, which `gate` does today, and three more variables). A near-duplicate of the action, not an exact copy: extend the action, make a second one, or write the job inline | **Extend the action** with the optional inputs `cell-job` and `cells`; the four existing callers are unchanged. No second action, no inline job |
| #269 (2026-11-17) | Split M5, the largest stage, in two? Four of its jobs are plain moves; `contract` is a matrix call three levels deep, and moving it moves the marker rule out of `gate` | **Two stages**: M5a the four plain jobs (`contract` and the marker rule stay), M5b `contract` and the cells' judging |
