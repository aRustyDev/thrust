# CI/CD

The repository has a CI pipeline and a deploy pipeline. One workflow, `ci.yml`, runs the test suites on every pull
request. It calls a second workflow once per contract cell, a third repeats a cell on demand and weekly, and a fourth
runs the browser journeys every day. A fifth, `deploy.yml`, deploys the Worker: a release candidate (`rc/*`) to staging
once `ci` passes on its pull request, and a release (`v*` tag) to production once `ci` passes on the tagged commit and the
reviewers approve, both while the repository variable `THRUST_DEPLOY_ENABLED` is `true`, and by hand from the same refs;
`main` never deploys. A sixth, `release-plz.yml`, keeps the next release's candidate pull request after every push to
`main`, and tags the release and opens it as a draft when one merges. A seventh, `release.yml`, builds the release's bundle
at the tag, attests it and an SBOM, attaches them and publishes the release ([Releasing](release.md)). No workflow publishes a
server image. The first half of this page is how CI is built and what its one required check guarantees; [The deploy
pipeline](#the-deploy-pipeline) is how the Worker gets to an environment, and the steps to run it are on [Deploying the
Worker](deploy.md). The jobs and the command each CI job runs are in the CI section of
[Running the suites](../testing/running.md#ci), and are not repeated here.

## What exists

| File | Does |
|---|---|
| `.github/workflows/ci.yml` (`ci`) | The pipeline. Runs on pull requests, merge-queue entries, pushes to `main`, every night on `main` and by manual dispatch |
| `.github/workflows/contract-cell.yml` | A reusable workflow that runs one contract cell on its own runner and uploads a marker that exactly that cell passed. Called by `test.yml`'s `contract` job, once per cell, and by `e2e.yml` |
| `.github/workflows/documentation.yml` (`documentation`) | The book's check, called by `ci.yml` (its `documentation` job) and runnable alone: the `docs` job (moved here unchanged, ADR-0102 and the migration, phase 1 of `.agents/plans/ci-workflows/`) and a `verdict` job that judges it against the plan it was given. `ci.yml`'s `gate` waits for the calling job and requires the `verdict` output |
| `.github/workflows/security.yml` (`security`) | The supply-chain check, called by `ci.yml` (its `security` job) and runnable alone: the `deny` job (moved here unchanged) and a `verdict` job that judges it against the plan it was given. `ci.yml`'s `gate` waits for the calling job and requires the `verdict` output |
| `.github/workflows/web.yml` (`web`) | The stock app's build check, called by `ci.yml` (its `web` job, which keeps the id the job always had) and runnable alone: the `web` job (moved here unchanged) and a `verdict` job that judges it against the plan it was given. `ci.yml`'s `gate` waits for the calling job and requires the `verdict` output. The workflow and its job share a name, so the checks are `web / web` and `web / verdict` |
| `.github/workflows/lint.yml` (`lint`) | The repository's guards and the Rust lint, called by `ci.yml` (its `lint` job, which keeps the id the job always had) and runnable alone: the `deps` and `lint` jobs (moved here unchanged), the `bash` job (shellcheck over the shell the repository maintains, through `check-shell.sh`; the Lint phase's first stage) and a `verdict` job that judges them against the plan it was given, which may name any of the three or all. `ci.yml`'s `gate` waits for the calling job and requires the `verdict` output. The workflow and one of its jobs share a name, so the checks are `lint / deps`, `lint / lint`, `lint / bash` and `lint / verdict`. `deps` runs the guards of these files too, `lint.yml` among them |
| `.github/workflows/test.yml` (`test`) | The repository's tests, called by `ci.yml` (its `test` job) and runnable alone: the jobs `ci-scripts` (the CI's own scripts: the planner, the gate, the lint, the mutant driver, the deploy and release helpers; its 47 commands run a core at a time under GNU parallel, the slowest first, and every one runs even when another fails), `hermetic` (the nextest loop, the contract counts, the doctests, the OpenAPI tests), `pg-adapter` (the Postgres and SMTP adapters against containers), `contract` (the contract cells: one call of `contract-cell.yml` per cell of the matrix `ci.yml` gives it) and `mutants` (the mutation controls: a matrix of four runners that each run their share of the 66 rows, because the rows cannot share a checkout; the id is unchanged, so the job's result is the four legs' aggregate) and `workers` (the Worker's wasm32 unit tests, [status](../testing/status.md) G-6: nextest on Node through `wasm-bindgen-test-runner`, which the job installs at the version `Cargo.lock` pins; new in stage 8 of the Test phase, the first five were moved here unchanged but for the mutants matrix), and a `verdict` job that judges them against the plan it was given, which may name any of the six or all, and that collects the cells' pass markers and proves that each planned cell reported. `ci.yml`'s `gate` waits for the calling job and requires the `verdict` output; it judges no cells itself. `ci.yml` calls it with the planner's `plan`, `matrix` and `cells`, and the cell calls are three levels deep (`ci.yml` to `test.yml` to `contract-cell.yml`) |
| `.github/workflows/build.yml` (`build`) | What the repository builds, called by `ci.yml` (its `build` job) and runnable alone (Phase 3 of `.agents/plans/ci-workflows/`, `phases/0004-build.md`): each job is a **compile proof** for one target, uploads what it built as a workflow artifact for three days, and publishes, signs and attests nothing (no secret, environment or `id-token`). Today the job `linux` builds `thrust-server` and `migrate` in release for `x86_64-unknown-linux-gnu` (its steps are one action's, `build-native`: a job gives it a name and a target, and it checks that the compiler's host is that target; the macOS build uses it too, from `build-extended.yml`, where it is not a required check), the job `wasm` builds the two Workers the bundle ships (the Worker as it ships, with `--no-default-features`, and the edge Worker) in release for `wasm32-unknown-unknown`, with the bundle's own commands, and a `verdict` job judges them against the plan it was given. `wasm` uses the cache key of the bundle's build (`deploy-release`) and saves it on `main`, so that the next release candidate or tag restores it: a cache is read by every ref and written by one, and the bundle's own runs (an `rc/*` branch, a tag) save none. The deploy bundle is not built here: `deploy.yml` and `release.yml` build it with the action `build-bundle` and attest it, and `verify-bundle.sh` requires those two files as its signers |
| `.github/workflows/flake-hunt.yml` | Repeats one cell 1 to 50 times (5 by default), by hand or every Monday at 03:17 UTC (`native-memory main`). It offers every pass of the matrix, and its first step, `.github/scripts/check-flake-hunt-cell.sh`, refuses a target and pass that are not a cell of `scripts/contract-all.sh`, before anything is built; `test-check-flake-hunt-cell.sh` holds the workflow's two lists equal to the matrix's. A pass is prepared by the same action a cell uses (`contract-pass-tools`). It passes only if `scripts/flake-hunt.sh` prints `failures: 0 / N`, because the script ends with an `echo`, so its exit status does not say whether an iteration failed. The job stops at 120 minutes: a `worker-cf e2e` iteration takes about 9 (nightly run 43798891735), so that cell fits about 12 |
| `.github/workflows/build-extended.yml` (`build-extended`) | The builds that are useful and are **not required checks**, by the owner's decision of 2026-11-23 ("lower priority checks that should run during either a pre-release check or nightly"): the job `macos` builds `thrust-server` and `migrate` in release for `aarch64-apple-darwin` on a hosted macOS runner (`macos-15`, free in a public repository), with the steps of the action `build-native`, and uploads them for three days. It runs every night (06:11 UTC, on `main`, which also saves its cache), when a release candidate is pushed (`rc/*`), by hand, and on a pull request that edits the file or the action `build-native`; no other pull request starts it. **Not gated**: `ci.yml` does not call it, `gate` does not read it, and a red run blocks neither a pull request nor a release candidate's staging deploy, so it has no plan and no `verdict` (as `deep.yml` has none). The first run, in #365, took 234 s with no cache |
| `.github/workflows/deep.yml` | The randomized tests (`prop_` and `model_`) at 4096 cases, every night at 05:37 UTC, by hand, and on a pull request that edits the file (so that it is proved before it merges): the `deep` nextest profile, less the `oracle_` tests, which need the TypeScript oracle's generated output and so run by hand. A job of 6.5 to 11 minutes in the four runs measured. **Not gated**: `ci.yml` does not call it, `gate` does not read it and a red run blocks nothing (Test phase, stage 3; the owner's decision of 2026-11-19, status G-48). It is red at random while issue #114 is open; any other red test is a finding |
| `.github/workflows/e2e.yml` | (Named `e2e-schedule.yml` until the last stage of the migration to one workflow per concern; the records of its earlier runs, in `testing/status.md` G-31, use that name.) Runs the `worker-cf e2e` and `worker-cf e2e-mail` cells (through `contract-cell.yml`, one job each) every day at 04:41 UTC and by hand, and on a pull request that edits the file, so that the call is proved valid before it merges. It covers what a pull request does not: drift in the runner, the browser and the packages, every day on `main`. Not a gate: nothing waits for it and `gate` does not read it. The pinned-Plane leg is not here, because the oracle builds from an unpushed fork (`scripts/e2e.sh plane-oracle`, local) |
| `.github/workflows/release-plz.yml` (`release-plz`) | The release flow, run by release-plz: after each push to `main`, opens or updates the release-candidate pull request from an `rc/*` branch, and tags `vX.Y.Z`, creates the GitHub release as a draft and starts `release.yml` on the tag when such a pull request merges. See [Release candidates](#release-candidates) |
| `.github/workflows/release.yml` (`release`) | Dispatched on the release tag by `release-plz.yml` (a draft release starts no workflow), or by hand on the tag; which run releases what is `plan-release.sh`. It builds the bundle at the tag, attests its build provenance and an SPDX SBOM of the source, attaches the bundle, the SBOM and `SHA256SUMS`, and publishes the release (immutable). On a pull request that edits it or the build, a dry run that attests and publishes nothing. See [Releasing](release.md) |
| `release-plz.toml` | release-plz's configuration: one version and one tag for the workspace, one root `CHANGELOG.md`, which commits make a release. Held to the workspace members by `.github/scripts/check-release-plz.py` |
| `.github/workflows/deploy.yml` (`deploy`) | The deploy pipeline: plan, build and attest a release candidate's bundle (staging) or take a release's (production), deploy it to an environment and prove it, roll back by itself when the smoke fails. Runs after `ci` on a release candidate or a tagged push to `main`, by hand, and on a pull request that edits its files (a dry run that touches no environment). See [The deploy pipeline](#the-deploy-pipeline) |
| `.github/workflows/preview-reconcile.yml` | Reconciles the per-pull-request database schemas and Hyperdrive resources of the previews, then publishes a receipt with no secret in it. On a pull request (opened, reopened, synchronize), every 15 minutes and by hand, in the `preview-control-plane` environment, and only while the repository variable `THRUST_PREVIEW_PROVISIONER_ENABLED` is `true`. Not a gate |
| `.github/workflows/preview-migration-source.yml` | Runs when `preview-reconcile` completes successfully for a same-repository pull request (a `workflow_run`, so GitHub runs the default branch's copy of the file, with `contents: read` and `pull-requests: read` and nothing else). Verifies the pull request's head and reads its migration files as untrusted Git data: the commits are fetched as Git objects only, never checked out, built or executed, and the artifact it uploads is untrusted data. Not a gate |
| `.github/workflows/preview-migration-verify.yml` | Runs when `preview-migration-source` completes, and by hand with the id of a completed source run (`workflow_dispatch`). A read-only rehearsal of the future privileged caller: it compares the untrusted source artifact with fresh Git objects and checks the pull request again before it reports success. No permissions. Not a gate |
| `.github/workflows/deploy-preview-provisioner.yml` | By hand, from `main` only: builds the preview provisioning Worker and deploys it without a public route (the `preview-provisioner` environment). Not a gate |
| `.github/workflows/enable-preview-provisioner.yml` | By hand, from `main` only: verifies Access, then enables the provisioner's route (the `preview-provisioner` environment). Not a gate |
| `.github/workflows/cloudflare-infrastructure.yml` | By hand: plans or applies the Cloudflare infrastructure with OpenTofu against the existing remote state, in the `cloudflare-infrastructure` environment; the apply uses the plan the protected run made. Not a gate |
| `.github/actions/contract-pass-tools` | A composite action: what one pass of `scripts/contract.sh` needs beyond Rust (`uv` for `oracle`; Node, pnpm, the app built from `web/` and Chromium for `e2e`, the browser alone and not the fonts `--with-deps` would apt-install, which took 8 minutes on a slow mirror), then a check that every host tool the pass relies on exists. `contract-cell.yml` and `flake-hunt.yml` both use it, so a pass one of them can run the other can too |
| `.github/actions/verdict` | A composite action: the judge at the end of a called workflow. It runs `assert-gate.sh` over the workflow's own jobs against the plan it was given (no planner job, no cells), and only after that passed sets the output `verdict=pass` that `ci.yml`'s `gate` requires. `documentation.yml`, `security.yml`, `web.yml`, `lint.yml` and `test.yml` call it from their `verdict` job; the job checks the repository out first, since a local action is read from the checkout |
| `.github/actions/setup-rust` | A composite action: the Rust version `rust-toolchain.toml` pins (read by `.github/scripts/toolchain-channel.sh`, which refuses anything but an exact version), the rustup targets and components, a keyed `rust-cache`, and optionally `worker-build` 0.9.0 and wrangler 4.151.2 |
| `.github/actions/build-native` | A composite action: a compile proof of the native server and `migrate`, in release, for the target of the runner it runs on. It checks that the compiler's host is the target the job claims (a job that landed on another runner fails instead of uploading binaries for the wrong target), builds both with the repository's commands (`migrate` with the one `build-bundle` uses; the wiring test holds the line equal), checks that both binaries exist, and uploads them for three days. Used by the job `linux` of `build.yml` and the job `macos` of `build-extended.yml`; the runner is the job's. It was written for `macos`, whose steps would have been an exact copy of `linux`'s |
| `.github/actions/build-bundle` | A composite action: the deploy bundle of the checked-out commit (the Worker, the `migrate` binary, the web app and the edge Worker, in one reproducible file), used by `deploy.yml` and `release.yml` so that both build it the same way |
| `.github/actions/deploy-worker` | A composite action: fetch the bundle (this run's build for staging, the release's asset for production, `fetch-release-bundle.sh`), verify its attestation, unpack it, install wrangler (the same version as `setup-rust`, kept equal by a test) and `cloudflared` at pinned versions, and run `scripts/deploy-worker.sh` |
| `.github/actions/triggerd` | A TypeScript action (its `dist/` is checked in) for a bounded, policy-based selection of the checks a change needs. Present and not enabled: the planner in use is `.github/scripts/plan-ci.py` |
| `.github/scripts/` | The pipeline's own logic: `plan-ci.py`, `plan-cells.sh`, `assert-gate.sh`, `assert-required-contexts.sh`, `lint-all-workflows.sh`, `lint-actionlint.sh`, `check-ci-index.sh` and `check-shell.sh` (shellcheck over the maintained scripts: the `bash` job's one command); and the deploy's: `plan-deploy.sh`, `classify-deploy.sh`, `active-version.py`, `assert-sha-checks.sh`, `assert-gated-environments.sh`, `verify-bundle.sh`, `unpack-bundle.sh`, `fetch-previous-assets.sh`, `fetch-release-bundle.sh` and `lint-workflow.sh`; and the release's: `plan-release.sh`. The `ci-scripts` job tests them with `.github/scripts/tests/` |
| `.github/workflows/README.md`, `.github/actions/README.md` | The indexes of the two directories: one row for each file, with its triggers and whether `gate` waits for it. `check-ci-index.sh`, in the `deps` job, fails when a file has no row, a row names no file, or a file has two |
| `.github/deploy/` | What the deploy trusts: the Cloudflare Origin CA root that signs `thrust-pg`'s certificate, with its fingerprint pinned by a test; and `flagship/policy.json`, who may reach which kind of target through the edge ([Routing flags](#routing-flags)) |
| `.github/workflows/flagship.yml` (`flagship`) | The edge's routing flags: makes every flag of the Flagship app what the policy says, on a push to `main` that changes it, and checks them every night at 05:23 UTC ([Routing flags](#routing-flags)). Off until the repository variable `FLAGSHIP_APP_ID` is set |
| `.github/workflows/edge-infrastructure.yml` (`edge-infrastructure`) | The edge's Cloudflare root, `infra/edge` (ADR-0111): the Flagship app and the Access applications (the opt-in paths, the Previews locked to the edge's token), in a state of its own. A pull request runs `tofu validate`, the contract test on a mocked provider, its guards (no flag or service token declared there; its state key used by no other workflow) and `service-token.py`'s tests, with no credential. A dispatch plans or applies, only once `edge-infrastructure` has required reviewers. The service tokens' secrets never enter its state: `infra/edge/service-token.py` makes them |
| `scripts/flagship.py` | The only writer of the routing flags: `validate`, `check`, `sync`, and `put`, `alias` and `delete` for the deploys and teardowns of targets. Tested by `.github/scripts/tests/test_flagship.py` against `flagship_fake.py` |
| `scripts/preview-config.py` | A Preview target's configuration (ADR-0111): production's rendered configuration, unchanged at the top level, with a `previews` block derived from it, because a Preview inherits none of production's bindings. The `shared` profile restates every binding, uses production's rooms (`script_name`), and leaves out the queue consumer and the cron, which never target a Preview. A setting it does not know is refused. Not run by a deploy yet (stage E of #195). Tested by `.github/scripts/tests/test_preview_config.py` |
| `scripts/deploy-worker.sh` and its parts | The deploy itself (render, classify, expand migration, `wrangler deploy`, smoke, rollback), run by the composite action: `render-wrangler.sh` (with the template `crates/worker/wrangler.deploy.toml`), `migrate-ci.sh`, `smoke-deploy.sh`, `smoke-bootstrap.sh`, `merge-assets.sh` and `check-asset-retention.sh` |
| `scripts/edge-opt-in.sh` | Opt in at the edge as CI (ADR-0111). It asks `/_edge/opt-in/<audience>?target=<name>` with the CI opt-in service token (from a file, never argv) and writes the edge's pin to a cookie jar, retrying while the target's flag propagates. It is for the smoke of a target: `smoke-deploy.sh` with `SMOKE_COOKIE_FILE` (the pin on every request) and `SMOKE_EXPECT_ROUTE` (the edge's `x-thrust-route` must name the target). `deploy-target.sh` runs it. Tested in `test-smoke-deploy.sh` against `smoke_stub.py` |
| `scripts/deploy-target.sh` | Deploy one target, a Worker Preview of the production Worker, and prove it through the shared hostname (ADR-0111). In order: production's configuration rendered and the target's derived from it (`preview-config.py`, shared data); production's live commit read, and the candidate classified against it: one that brings migrations is deferred (nothing done) to a job with reviewers and the database route, where `ALLOW_MIGRATE=true` runs `migrate-ci.sh --no-contract` first; `wrangler preview`; its answer read by `preview-result.py` (the URL the edge accepts, the deployment id, the bindings as Cloudflare applied them, the rooms on production's Worker); the flag written (`flagship.py put`); the opt-in and the smoke through the edge, then the smoke's control; with `TARGET_ALIAS`, that alias (`staging`) pointed at the target last. A failure from the Preview on deletes the flag, then the Preview. Each tool sees only its own secret. `deploy.yml`'s `deploy-target` and `deploy-target-migrate` run it, off until the cut-over (`THRUST_TARGETS_ENABLED`). Tested by `test-deploy-target.sh` (fakes: `target_fakes.py`, the real `flagship.py` on `flagship_fake.py`) and `test_preview_result.py` |
| `scripts/deploy-edge.sh` and `scripts/edge-controls.sh` | Deploy the edge Worker from the release's bundle and prove it on the shared hostname (ADR-0111). In order: the edge's template rendered (`render-wrangler.sh`); its live version read (a split or unreadable one stops the deploy); `wrangler deploy --tag <sha>` with the edge's three secrets in a `--secrets-file` only its owner can read, removed afterwards; the first deploy stops there, green, because the edge has no hostname until the cut-over moves `thrust.sow.foo` to it. After that come the smoke through the edge (`SMOKE_EXPECT_EDGE`: this edge answers, `SMOKE_EXPECT_ROUTE=prod`, production at the release, and a real read), the smoke's control (an edge nobody deployed must fail), and `edge-controls.sh`: a forged pin goes to production, and the opt-in gives no pin without Access. Any failure rolls back to the edge that was live, proves it answers again, and leaves the run red. `deploy.yml`'s `deploy-edge` runs it after production, off until the cut-over (`THRUST_EDGE_ENABLED`). Tested by `test-deploy-edge.sh` (fakes: `edge_fakes.py`) and `test-smoke-deploy.sh` (`smoke_stub.py`) |
| `scripts/check-web-diff.sh` | The Q-R2 gate for `web/` (the stock React app, imported from the oracle pin), run by the `web` job (in `web.yml`). Tested by `.github/scripts/tests/test-check-web-diff.sh` |

## One required check

The ruleset on `main` requires exactly one status check, `gate` `[verified 2026-10-19 fa4bd6c]`. Every other job is
evidence that `gate` inspects. `gate` runs even when a job before it failed, and `.github/scripts/assert-gate.sh`
decides:

- The `changes` job must have succeeded. It names the jobs that must run (`jobs`), what the gate itself expects (`top`: the
  workflows it calls that have a planned job; no job is left in `ci.yml` for the gate to judge itself), each called workflow's
  jobs (`plans`) and the contract matrix and cells (`matrix` and `cells`, which `ci.yml` hands to `test.yml`), from
  `.github/scripts/plan-ci.py` (see [What runs when](#what-runs-when)).
- Every expected job must be exactly `success`. A skipped job is a failure here, not a pass.
- Every job that is not expected must be `skipped`. One that ran anyway means its `if:` and the planner disagree.
- The planner and the gate list the same six names: the workflows `ci.yml` calls. An expected job
  that is not a need of the gate, or a need the planner does not know, fails it, as does an input that is missing or is not
  a JSON array of strings.
- **The contract cells are judged by `test`'s own `verdict` job, not by the gate** (they were the gate's until the job
  `contract` moved into `test.yml`). `contract` is expected exactly when cells are planned, and the set of cells that uploaded a
  pass marker must then equal the planned set, which must not be empty. The `verdict` action downloads the markers (the
  artifacts `cell-ok-*` of the same run, across the nested call) when it is given a cell job, and tolerates a failed download:
  with no marker the judge fails on `no cell markers`, so it cannot pass.
- `test-ci-scripts.sh` generates the cases: each of the six names, expected or not, with each of the four results (48
  cases), then the inputs and the names; and, at the level of a verdict job whose cell job is `contract`, each of the six jobs of
  `test.yml` the same way, and the cells' cases (a planned cell that never reports, a marker for an unplanned cell, no cell
  planned, no marker, cells planned and the cell job not expected, the inputs only a workflow with cells has).
- **A called workflow is judged twice** (ADR-0102). `assert-gate.sh` also takes `KNOWN_JOBS`, `CHANGES_JOB`, `CELL_JOB` and
  `VERDICT_JOBS`, each of which defaults to what the gate above needs (the cell job to none: a level that has cells must say so.
  A level that names no cell job and is given cells anyway is refused, so a verdict job that forgets its cell job fails on every
  run that plans a cell, since the action passes the cells through; the wiring test holds `test.yml` to say it on every run). The called workflow's own `verdict` job calls the
  `verdict` action, which sets them to judge the workflow's jobs against the plan it was given; the gate lists the called
  workflows as `VERDICT_JOBS`, which requires of each
  expected one that its result is `success` **and** that its output `verdict` is `pass`, since a workflow whose `verdict` job
  was skipped is `success` and proves nothing. `VERDICT_JOBS` is a static list in `ci.yml`, not the planner's output, so a
  planner that dropped a workflow could not switch its check off; `test_workflow_wiring.py` holds it equal to the planner's
  `WORKFLOWS`. Today six workflows are called, `documentation.yml` (the `docs` job), `security.yml` (the `deny` job),
  `web.yml` (the `web` job), `lint.yml` (the `deps`, `lint` and `bash` jobs), `test.yml` (the `ci-scripts`, `hermetic`, `pg-adapter`, `contract`,
  `mutants` and `workers` jobs) and `build.yml` (the `linux` and `wasm` jobs), so a plan may name only some of a workflow's jobs: `clippy.toml` plans `lint` alone, `.shellcheckrc` plans `bash` alone, a guard of `deps` plans `deps`
  and `bash` (as does every shell script), a deploy script plans `ci-scripts` and `bash`, and `e2e/` plans `contract` alone. The cases (221 in `test-ci-scripts.sh`) include the seven scenarios of the gate spike
  (`.agents/plans/ci-workflows/research/0018-gate-spike.md`), each called workflow tried on its own.
- **The wiring is tested against the real files.** `test_workflow_wiring.py` (239 checks, for six workflows) holds `ci.yml`, the
  planner and each called workflow's file to each other: the calling job (`needs: changes`, it runs when `top` names it, it
  passes its own plan from `plans`), the gate's needs and its `VERDICT_JOBS`, the workflow's jobs and its `verdict` job
  (`always()`, checking out and calling the `verdict` action with its needs, its plan and its jobs, and running nothing of
  its own), and no workflow-level `concurrency` or `github.event_name` in a called workflow. The action is checked once: its
  judge (`assert-gate.sh`, no planner job), its collection of the cells' markers (only when given a cell job, into the directory the
  judge reads) and its output, set only after the judge. The workflow that holds `contract` is checked for the matrix and the cells it
  is given, its call of `contract-cell.yml`, and the cell job and cells it passes to the action; `ci.yml` for passing them, and its
  gate for judging no cells; and `ci.yml` for holding only `changes`, the calls and `gate`.
- **Tested to be able to fail.** `.github/scripts/tests/mutate_gate.py`, one of the commands of the `ci-scripts` job, plants 379 hand
  mutants one at a time in `plan-ci.py`, `assert-gate.sh`, `ci.yml`, the called workflows and the `verdict` action, and each
  must fail the tests of its file (`test_plan_ci.py` and `test_workflow_wiring.py`; `test-ci-scripts.sh` under dash, stopping
  at the first failing case). The mutants of a called workflow are written once and made for each of them (and for each of its jobs, a workflow of several
  jobs getting the mutants only several can have). It
  refuses to answer when the tests fail on the unmutated files, when the text a mutant replaces is not found exactly once, or
  when a mutant no longer compiles, and fourteen mutants that change nothing must survive, so a driver that kills everything is
  caught. `[verified 2026-11-23 locally, under dash: 379 killed, 14 survive]`

The point is that "nothing failed" is not accepted as "everything ran". A cell that never started leaves no check
behind, and a required check that never reports blocks a pull request forever.
`assert-required-contexts.sh` applies the same idea from outside: it certifies a pull request by the required set of
its base branch. Nothing calls it, so it is a tool to run by hand.

## What runs when

- **Affected only.** The `changes` job diffs the change against its base and runs `.github/scripts/plan-ci.py` over the
  changed paths, Cargo's graph of the workspace (`cargo metadata --no-deps`) and the matrix. A path's first matching rule
  decides; the rules, and why each exists, are in the script.
  - A crate's files change that crate, and every crate that depends on it, through any kind of dependency, is affected.
    A change to any crate runs `ci-scripts`, `deps`, `lint` and `hermetic` (they build the whole workspace). A change to
    its `Cargo.toml` runs `deny` too.
  - A cell's target runs all its passes when the binary it starts is affected (`native-*` start `thrust-server`,
    `worker-*` start `thrust-worker`), or when the suite itself, `thrust-contract-tests`, changed. `pg-adapter` runs when
    `thrust-adapters-selfhosted` is affected. `mutants`, which mutates the native server, runs when `thrust-server` is
    affected or the suite changed. `workers`, the Worker's wasm32 unit tests, runs when `thrust-worker` is affected,
    and `scripts/wasm-tests.sh`, `scripts/wasm-test-runner.sh` and `scripts/lib/workerd-stubs.cjs` run it with
    `ci-scripts`. `linux`, the release build of the native server and of `migrate` (in `build.yml`), runs when
    `thrust-server` or `thrust-adapters-postgres` is affected; `wasm`, the release build of the Worker and of the edge
    Worker (also in `build.yml`), when `thrust-worker` or `thrust-edge` is.
  - Some files outside the crates are read by the crates' tests, which Cargo's graph cannot see, so they run those tests'
    jobs:
    - `tools/oracle-v1/` (its schema and corpus are read by the tests of four crates) runs `hermetic`, `deps` (the
      OpenAPI diff) and the three `oracle` cells.
    - `.github/deploy/` (read by the TLS tests of `thrust-adapters-postgres`) runs `hermetic` and `ci-scripts`.
  - A file that only one pass reads runs that pass on every target:
    - `e2e/`, `scripts/e2e.sh` and the e2e tools run `worker-cf e2e` and `worker-cf e2e-mail` (the two suites share `e2e/support/`
      and the runner, so a change to either is not told apart).
    - `web/` runs the `web` job (in `web.yml`) and both e2e cells. A change to `web/` alone is now driven in a browser on its own pull
      request (G-30's gap 2).
  - A file that only a guard or a job reads runs that job:
    - `deny.toml` runs `deny`, which lives in `security.yml` and is called by `ci.yml`; `clippy.toml` and `rustfmt.toml` run `lint`, which lives in `lint.yml` and is called by `ci.yml`.
    - `release-plz.toml` and the guards of `deps` run `deps`, which lives in `lint.yml` too.
    - A deploy script (`smoke-deploy.sh`, `migrate-ci.sh`, …) runs `ci-scripts` (and `bash`, below).
    - Another workflow, action or helper script runs `ci-scripts` and `deps` (the workflow lint).
    - A shell script (any `*.sh` outside `web/`; `scripts/rs`, which has no extension, runs everything), `.shellcheckrc` or a composite action's `action.yml` (its `run:` blocks are shell) also runs `bash`, besides what the rule for that script says: shellcheck reads what `check-shell.sh` lists, and the planner does not repeat its list of what is left out (a plan's script, a spike's), so a change to one of those plans a job that finds nothing to say.
  - Everything runs on a path whose readers the planner does not know:
    - the workspace manifest, the lockfile and the toolchain;
    - `.cargo/` and `.config/`;
    - `ci.yml`, `contract-cell.yml`, the called workflows (`documentation.yml`, `security.yml`, `web.yml`, `lint.yml`, `test.yml`, `build.yml`), `setup-rust`,
      `contract-pass-tools` and `verdict`;
    - the planner and the gate;
    - the scripts that run a cell;
    - an unknown directory under `crates/`;
    - an unlisted script under `scripts/`.
  - The book's inputs run the `docs` job, which lives in `documentation.yml` and is called by `ci.yml`: `docs/`, `AGENTS.md`
    (the book includes its testing rules), `.config/branding/` (its palette) and `scripts/check-book.sh`.
  - A path that no rule matches needs nothing: `README.md`, `.agents/`, `probes/` (which only `scripts/verify-all.sh`
    runs, locally) and the other `tools/`. A pull request of only such paths runs `changes` and `gate`, and `gate` passes.
- **Everything runs** in these cases:
  - a release candidate's pull request;
  - a merge-queue entry;
  - the nightly run of `main` (03:23 UTC), which catches a new RustSec advisory or drift in the runner, though no
    change brought it;
  - a manual run;
  - whenever the planner cannot answer: no base commit, a failed diff, an empty change list, or a failed `cargo metadata`.

  A target of `contract-all.sh` that the planner does not know runs on every change. `deploy.yml` ignores the nightly
  run: `plan-deploy.sh` deploys only after a pull request or a push.
- **Examples** (the planner run on this repository) `[verified 2026-11-23 locally]`:

  | Change | Jobs | Cells |
  |---|---|---|
  | a file under `web/` | `web`, `contract` | 2 (`worker-cf e2e`, `worker-cf e2e-mail`) |
  | `thrust-adapters-cloudflare` | `ci-scripts`, `deps`, `lint`, `hermetic`, `pg-adapter`, `contract`, `workers`, `wasm` | the 16 `worker-*` cells |
  | `crates/worker/src/lib.rs` | `ci-scripts`, `deps`, `lint`, `hermetic`, `contract`, `workers`, `wasm` (its `Cargo.toml` adds `deny`) | the 16 `worker-*` cells |
  | `crates/adapters/selfhosted/Cargo.toml` | `ci-scripts`, `deps`, `lint`, `deny`, `hermetic`, `pg-adapter`, `contract`, `mutants`, `linux` | the 10 `native-*` cells |
  | `crates/server/src/lib.rs` | `ci-scripts`, `deps`, `lint`, `hermetic`, `contract`, `mutants`, `linux` | the 10 `native-*` cells |
  | `thrust-contracts` (everything depends on it) | all but `deny`, `web` and `docs` | all 26 |
  | `tools/oracle-v1/` | `deps`, `hermetic`, `contract` | the 3 `oracle` cells |
  | `scripts/wasm-tests.sh` (or the runner and stand-ins beside it) | `ci-scripts`, `workers` | none |
  | `scripts/smoke-deploy.sh` | `ci-scripts` | none |
  | a page under `docs/` | `docs` (in `documentation.yml`) | none |

- **Tested** by `test_plan_ci.py`, about 7,000 checks:
  - 300 generated crate graphs, with each crate changed alone and in pairs, against an independent reverse-reachability;
  - the plan of a list equals the union of the plans of its parts;
  - every rule is the first match of its own sample, and matches a tracked file;
  - the fail-open cases;
  - this repository's real graph and targets;
  - no script that the planner keeps out of the cells is called by what runs a cell;
  - the called workflows (ADR-0102): 300 generated partitions of the jobs into workflows against an independent computation
    of what `gate` expects and what each workflow is given, the maps that are refused (a job in two workflows, a name that is
    no job, a workflow with no job), and the command line run on a copy of the planner whose map is not empty;
  - every file under `.github/workflows/` is one that runs the checks (planned as everything) or one that runs none, so a new
    workflow cannot fall into the generic rule and run no test.

  `[verified 2026-11-16 locally; run by ci-scripts]`
- **The web job.** The `web` job (in `web.yml`, called by `ci.yml`) runs `scripts/check-web-diff.sh` (only the files in
  `web/ALLOWED-CHANGES` may differ from the pin; `web/SOURCE` says where `web/` came from), then
  `pnpm --dir web install --frozen-lockfile --ignore-scripts` and `pnpm --dir web exec turbo run build --filter=web`,
  and checks that `web/apps/web/build/client/index.html` exists. Node is `web/.node-version`; pnpm is the version
  `web/package.json` pins, read from it. That install and build ran on the import as it is, with pnpm 11.10.0, ten
  tasks uncached from an empty `node_modules` `[verified 2026-11-11 locally; not yet in CI]`.
- **The matrix.** The `changes` job reads the `run <target> <pass>` lines of `scripts/contract-all.sh`
  (`plan-cells.sh`), so no second list can drift, and the planner keeps the cells the change needs. It fails if the number of cells it parsed differs from the number of lines that start with `run`,
  if there are none, or if one repeats. There are 26 cells: 23 contract cells (the browser journeys' `worker-cf e2e`, the ones that need mail, `worker-cf e2e-mail`,
  and the password reset by email's three `mail` cells, `native-memory`, `worker-memory` and `worker-cf`, among them) and the 3 `oracle` cells below `[verified 2026-11-14: PR #154's
  check list had 22 contract jobs, all success, before the first mail cell; the 21 of PR #113 (2026-11-10) were before the e2e cell; `e2e-mail` was added 2026-11-16]`.
  Each runs on a fresh runner with its own checkout, which is why the shared-build-directory hazard that makes
  `contract-all.sh` run serially does not apply.
- **A cell.** `contract-cell.yml` prepares the pass through `contract-pass-tools`, which checks that `curl`, `openssl`, `lsof`, `pgrep` and `docker` exist (and `nc` for the native `mail` cell, which waits for its Mailpit with it, `uv` for `oracle`, and `node` and `pnpm` for `e2e`), runs
  `scripts/contract.sh <target> <pass>` with no pipe so the step's status is the script's, and uploads a marker named
  for the cell. The script's target guard and count guard apply as they do locally. Every port the script binds (the target, the mock
  identity provider, Postgres, Mailpit) is drawn by `scripts/lib/ports.sh` from below the kernel's outgoing-connection range, which
  the script prints first (D-20 in the testing status).
- **The mail cells.** `native-memory mail` starts a Mailpit container (pinned, pulled from Docker Hub as `postgres` is) beside the native server, which is
  given `THRUST_SMTP_*` and `THRUST_PUBLIC_BASE_URL`, and runs `tests/mail.rs`: the password reset by email through a mail server written by someone else.
  `worker-memory mail` and `worker-cf mail` (E3) run the same seven tests (E4 added the four of the sign-in with a code) against the Worker built for wasm32 and run by `wrangler dev`, whose
  `send_email` binding is wrangler's own simulator: nothing is sent, and the message the binding was given is read from the dev server's log and the files
  it names. No container; the second also has the Postgres container every `worker-cf` cell has.
  The `pg-adapter` job also runs the SMTP adapter's Docker suite (`--test smtp_sink`, Mailpit again, with TLS and a login); the adapter's other two suites
  are hermetic and run in `hermetic`.
- **The oracle cells.** `native-memory oracle`, `native-pg oracle` and `worker-cf oracle` run the v1 oracle (issue #58,
  `tools/oracle-v1/README.md`) against the started target: the scoped Schemathesis gate, the corpus comparison, the
  comparison's own controls and the must-fail controls C0 to C4, ending in `VERDICT: PASS`. They are `run` lines of
  `contract-all.sh` like any cell, so the plan, the matrix, the pass marker and `gate` treat them as one, and the count
  guard does not apply: success is the script's verdict, which exits 1 on any failing step. They need `uv`, so the
  workflow installs it for that pass only (`astral-sh/setup-uv`, uv 0.12.18) and the preflight checks for it. Schemathesis
  (4.31.1) and the Python dependencies of the harness's scripts are fetched from PyPI at run time. The oracle's logs go
  to the cell's `logs-<target>-oracle` artifact. On hosted runners all three passed `[verified 2026-11-10 e591b29]`, so the
  action installed uv, the preflight found it and `uvx` fetched Schemathesis there; as whole jobs, checkout, toolchain and
  build included, they took 1m36s (`native-memory`), 2m11s (`native-pg`) and 4m00s (`worker-cf`), in parallel, against a
  40 minute job timeout.
- **Concurrency.** A new push cancels the older run of a pull request. Runs on `main` and in the merge queue are
  never cancelled. `deploy.yml` does the opposite on purpose: one deploy per environment at a time, and a queued deploy is
  never cancelled for the running one, because `migrate` has no lock.

## The deploy pipeline

`deploy.yml` is ADR-0110's round-29 pipeline: expand migration, `wrangler deploy` at 100 %, the smoke against the live
hostname, and an automatic `wrangler rollback` when it fails. There are no stages: at about two requests a second a 10 %
slice is a hundred requests, mostly the owner's, so the smoke is the gate. The steps to set an environment up and to
operate it are on [Deploying the Worker](deploy.md); this section is how it is built.

| Job | Holds | Does |
|---|---|---|
| `plan` | no secret | Which event and which commit (`plan-deploy.sh`, tested over every combination of its inputs). `ci` passing on a release-candidate pull request (head `rc/<name>` in this repository) plans a staging `dispatch`, and `ci` passing on a push to `main` whose commit carries exactly one `v<version>` tag plans a production `dispatch` on that tag, only while `THRUST_DEPLOY_ENABLED` is `true`. A manual run deploys the commit it runs from: `staging` from an `rc/<name>` branch, `production` from a `v<version>` tag whose commit is on `main`; the automatic dispatch also passes the commit it expects, and a branch that has moved since plans `skip`. A pull request is a dry run. The commit's own `gate` must have passed, asked for by name on that commit (`assert-sha-checks.sh`), and the environments that gate the deploy must have required reviewers (`assert-gated-environments.sh`: GitHub creates an unprotected environment when a job names one that does not exist) |
| `dispatch` | `actions: write`, no secret | Starts `deploy.yml` again with `workflow_dispatch` ON the `rc/*` branch or the `v*` tag, for the commit `ci` passed. GitHub checks an environment's branch rule against the run's own ref, and a `workflow_run` always runs on `main`, so a deploy restricted to `rc/*` or `v*` has to be its own run on that ref. A dispatch is one of the two events a workflow's own token may start a run with |
| `build` | no secret | Staging and a dry run only: the Worker, the `migrate` binary, the web app (`web/`) and the edge Worker (`crates/edge`), into one bundle (`.github/actions/build-bundle`), attested (SLSA provenance) in a deploy. Production builds nothing: its bundle is the release's, built once at the tag by `release.yml` |
| `dry-run` | none | For a pull request that edits the deploy files: the bundle is built and `wrangler deploy --dry-run` checks the rendered configurations, the Worker's and the edge's. No environment, no secret |
| `deploy` | the environment's secrets | Fetch the bundle: this run's artifact for staging; for production the release's asset, only from a published, immutable release and only the file GitHub's release attestation names (`fetch-release-bundle.sh`). Verify its attestation (signed on exactly this run's ref by `deploy.yml` for an `rc/*` branch, by `release.yml` for a `v*` tag; `verify-bundle.sh` refuses any other ref without asking), render the configuration, read what is live, classify the change, run the expand migration only if migration files changed, keep the previous release's web files, `wrangler deploy --tag <sha>`, the smoke, and its controls. It defers, deploying nothing, whatever cannot be rolled back across |
| `deploy-do` | the same secrets, behind reviewers | What `deploy` deferred: the first deploy to an environment, any change to `[[migrations]]`, a live state that cannot be read, a previous release's web files that cannot be found. No automatic rollback |
| `deploy-target` | `target`'s secrets, no database route | A release candidate as a target (ADR-0111, `deploy-target.sh`), while `THRUST_TARGETS_ENABLED` is `true`; one that brings migrations is deferred, with nothing done ([Release candidates as targets](deploy.md#release-candidates-as-targets-off-until-the-cut-over)) |
| `deploy-target-migrate` | `target-migrate`'s secrets, behind reviewers | What `deploy-target` deferred: production's schema expanded (`--no-contract`), then the target deployed |
| `deploy-edge` | `edge`'s secrets | After a production deploy that succeeded, while `THRUST_EDGE_ENABLED` is `true`: the edge Worker from the release's bundle, proved on the shared hostname and rolled back when that fails (`deploy-edge.sh`, [The edge's deploy](deploy.md#the-edges-deploy-off-until-the-cut-over)) |

The controls, each with a test that makes it fail:

- **The smoke and its control.** `scripts/smoke-deploy.sh` needs the version that answers (`x-thrust-version`) to be the commit
  just deployed, and a real read to work. The same smoke is then asked for a version nobody deployed and must refuse it: a
  smoke that passes that cannot tell one version from another, and the run fails on it.
- **Rollback.** When either fails and a rollback is possible, `wrangler rollback <previous version>` runs and the
  unauthenticated probe must show the previous version answering. The run stays red either way, and the database keeps the
  expanded schema (expand/contract: the previous code runs on it).
- **The manual gate.** The first deploy carries `[[migrations]] v1`, and a Durable Object lifecycle change can be neither
  gradual nor rolled back across. They go through `deploy-do`, and so does a deploy the classifier cannot place against what is
  live: a deploy of older code, a live commit it could not fetch (it asks GitHub for it by its sha, since release-plz drops it
  from the release candidate's branch), or a checkout too shallow to say. The classifier compares the two commits by what they
  hold, so a rewritten branch with no migration or Durable Object change in it is not stopped.
- **The previous release's web files.** A hashed file the new release dropped is answered `200 text/html`, the single-page
  fallback, not 404 (ADR-0011, Q-0011.1), and a 100 % deploy has no version affinity to hide it. So the deploy puts the
  previous release's files under its own (`merge-assets.sh`; the previous release's own build, so one release back), taken
  from that release's attested bundle, and afterwards asks the live hostname for a script only the previous release had
  and requires it to be served as that script (`check-asset-retention.sh`).
- **Secrets reach only what needs them.** wrangler gets the Cloudflare token; the expand step gets the database's and the
  tunnel's; the smoke gets its own account; the GitHub token reaches only the step that fetches the previous bundle.
  `test-deploy-worker.sh` has each fake report what it could see.

Its tests run in `ci-scripts` and `deps`, under dash: `deploy-worker.sh` runs against a fake Cloudflare that keeps state
(what the edge serves), so a rollback verifies only if it really made the previous version answer. Each script's own test
has a case that must fail beside each that must pass, and each script was also broken by hand, one way at a time, to see
its test fail.

**Run for real.** The deploy has run on staging from `rc/*` branches since 2026-11-15, and
[Deploying the Worker](deploy.md#what-the-first-staging-deploys-settled) lists what each run settled: the first deploy through
the manual gate, the shapes of `wrangler deployments list --json` and `versions list --json`, the queue consumer with its
dead-letter queue, the attestation flow end to end, the read of an environment's protection rules, the route through the Access
tunnel with `verify-ca` meeting a real Origin CA certificate, the keeping of the previous release's web files over two
releases, and the rollback drill (run 43811076183: the injected failure put the previous version back and ended the run red).
**Production has had its first deploy** (2026-11-16, release `v0.2.0`, run 43921084528: [what it settled](deploy.md#what-the-first-production-deploy-settled)). **Not yet run:** a second production deploy, the pipeline's own smoke against production (the first production smoke was run by hand), and a narrower token than the one in use. **Found by those runs, and changed since:** release-plz
force-updates the release candidate's branch, so the live commit is usually not in the next candidate's history, and the
classifier, which failed closed on that, sent two deploys to the manual gate with nothing in them that needed one (runs
43796746005 and 43803593395). It now fetches the live commit by its sha and compares the two commits by content, and still
defers a deploy of older code than what is live; tested here, not yet run on staging ([What the deploy compares with what is
live](deploy.md#what-the-deploy-compares-with-what-is-live)).

**The environments' branch rules** say the same as `plan`, so a run from anywhere else cannot reach an environment's
secrets even if the workflow were wrong: `staging` and `staging-do-lifecycle` take only `rc/*` branches, `production` and
`production-do-lifecycle` only `v*` tags (set by the owner; [Deploying the Worker](deploy.md#setting-an-environment-up)).
The previous release's bundle is looked for among artifacts of this repository's own runs (not a fork's) on an `rc/*`
branch or a `v*` tag, and verified on the ref its run was on (`fetch-previous-assets.sh`).

## Release candidates

[release-plz](https://release-plz.dev) 0.4.3 runs the release flow, configured by `release-plz.toml` and run by
`release-plz.yml` on every push to `main`. Nothing in it is hand-written except the guard that holds the configuration to the
workspace (below).

- **The release-candidate pull request** (`release-pr` job). When something releasable landed since the last `v*` tag,
  release-plz opens a pull request from a branch `rc/<timestamp>`, or force-updates the open one, titled
  `chore(release): vX.Y.Z`. Its commit sets `[workspace.package] version` in `Cargo.toml`, the workspace packages in
  `Cargo.lock`, and adds the new section to `CHANGELOG.md` (release-plz's default format, Keep a Changelog). It is made
  through the GitHub API, so GitHub signs it. The branch name carries a timestamp, not the version: release-plz takes only a
  prefix (`pr_branch_prefix`).
- **The release** (`release` job). When a push to `main` is the merge of a pull request from an `rc/` branch, release-plz
  tags the merge commit `vX.Y.Z` and creates the GitHub release, with that version's changelog section, as a draft
  (`git_release_draft`); `release.yml` attaches its assets and publishes it ([Releasing](release.md)). Any other push is
  a no-op (`release_always = false`). Squash merge, the repository's only merge method, makes the merge commit the one
  tagged.
- **One version, one tag, one changelog.** Every crate is in one `version_group` and inherits the workspace version; the
  tag and the release are made for `thrust-core` only, under `v{{ version }}`; `CHANGELOG.md` is written for
  `thrust-core` with every other crate's commits included. release-plz has no workspace-level version group, so
  `.github/scripts/check-release-plz.py` (run by `deps`, tested in `ci-scripts`) fails when `release-plz.toml` and the
  workspace members differ, a crate does not inherit the workspace version, or there is not exactly one tagger.
- **A new crate.** Every crate finds its previous version from the one `v*` tag, and release-plz 0.4.3 then looks for the
  crate in that tag's workspace; a crate added since the last release is not there, and `release-pr` stops with
  `cannot find package` (it did on `main` after `thrust-edge`, #211, landed after v0.2.0). So a new crate's entry gets a
  first-release pattern, `git_tag_name = "<crate>-v{{ version }}"`, which matches no tag: release-plz treats the crate as
  a first release, the commits under its directory go into the changelog, and they do not set the bump. Once a release
  contains the crate the pattern must go, or every later release lists the crate's whole history again. `check-release-plz.py --first-release` holds both ends against the highest `v<major>.<minor>.<patch>` tag:
  `required` in `deps` (which fetches the tags), so the pull request that adds a crate adds its pattern; `exact` in the
  `release-pr` job before release-plz runs, which fails from the release that contains the crate until the pattern is
  removed. `exact` is not in `deps` because it would fail every pull request in that window, and `main`'s run of the release
  commit itself.
- **What makes a release.** A commit that touches a crate's files and is a fix, a feature, performance work, a refactor, a
  revert, or a breaking change of any type (`release_commits`). A test, docs or chore commit does not. In 0.x a feature or a
  breaking change bumps the minor and a fix the patch (`features_always_increment_minor`). The first release is `v0.1.0`,
  the version `Cargo.toml` holds.
- **What does not: a change outside the crates.** release-plz looks at the files each crate packages, so a change under
  `web/` (which ships in the deploy bundle), `scripts/` or `e2e/` alone never opens a release-candidate pull request. To
  release one, the owner (an `rc/*` bypass actor) does it by hand: a branch `rc/manual-<version>` from `main`; the one
  `version = "..."` line of `[workspace.package]` set to the next version; `cargo update --workspace` (it moves every
  workspace package in `Cargo.lock`); a section for that version at the top of `CHANGELOG.md`; a pull request titled
  `chore(release): vX.Y.Z`; merge it. The `release` job tags and publishes it like any other. (`release-plz set-version`
  is not the tool for this: it writes a version into the one crate's own `Cargo.toml`, out of the shared version
  `[verified 2026-11-15 on a copy of this workspace]`.)
- **The App's token.** Both jobs act as the release GitHub App: a branch, tag or release made with `GITHUB_TOKEN` starts no
  workflow, so `ci` would not run on the `rc/*` branch and nothing triggered by the tag would start.
- **Staging.** The release-candidate pull request runs `ci` in full, whatever its diff (the `changes` job), and a green run
  dispatches the staging deploy of that commit on its branch ([The deploy pipeline](#the-deploy-pipeline)).

How each setting was checked: on a copy of this workspace with release-plz 0.4.3 `[verified 2026-11-15]`. Right after a
`v0.1.0` tag nothing is proposed; a fix in one crate proposes 0.1.1, rewrites `Cargo.toml`'s one version line, keeps
`Cargo.lock` consistent (`cargo metadata --locked` passes) and lists only that fix; a feature then proposes 0.2.0; a
breaking `chore!:` proposes 0.2.0; test and docs commits alone propose nothing; a fix only under `web/` proposes nothing.
Two settings the docs do not settle were read in the source instead: `release_always = false` matches the configured
`pr_branch_prefix` (the docs still name `release-plz-`), and a tag pattern set only on `thrust-core` made every other crate
look for a tag of its own and re-release all of history, which is why `git_tag_name` is set for the whole workspace.

The first-release pattern, with release-plz 0.4.3 on clones of `main` `[verified 2026-11-16]`. The code (`next_ver.rs`,
`process_git_only_package`) treats a crate as a first release only when no tag matches its pattern.

- At the merge of #218: without the pattern, `release-plz update` exits 1 with `cannot find package "thrust-edge"`, as
  the `release-pr` job did on `main`. With it, it proposes 0.2.1 with #211's feature in the changelog, and the
  candidate's `Cargo.lock` passes `cargo metadata --locked`.
- At the merge of #221, which moved every crate (ADR-0009): the same failure without the pattern. With it, nothing is
  proposed. release-plz counts a crate's commits by its current directory, so #211's feature, made under
  `crates/thrust-edge/`, no longer counts, and the move is a `chore`. The other crates are found in v0.2.0 by name,
  wherever they now live. #211's feature reaches a changelog only through a release made by hand (above).
- Then a `fix(edge)` commit proposes 0.2.1. With v0.2.1 tagged and a second fix: keeping the pattern lists the first fix
  (and the move) again in 0.2.2's section, and `--first-release exact` refuses it. With the pattern removed, 0.2.2
  lists only the second fix.

**Live since 2026-11-15.** The owner set up:

1. a GitHub App (no webhook; Contents and Pull requests: read and write), installed on this repository only;
2. the environment `release`, deployment branches limited to `main`, with the secret `RELEASE_APP_PRIVATE_KEY` (the App's
   private key), and the repository variable `RELEASE_APP_CLIENT_ID` (the App's client id), which turns both jobs on;
3. a ruleset on `rc/*` (create, update, delete and force-push refused to all but the App and the admin role), a ruleset on
   `v*` tags (create, update and delete refused to all but the App), and immutable releases;
4. on `main`, a pull request (no approvals, squash only), no force-push and no deletion, beside the required `gate`.

The first run as the App `[verified 2026-11-15]` opened #184, `chore(release): v0.1.0`, from `rc/2026-11-15T07-33-32Z`:
one commit that GitHub shows as verified, adding the `0.1.0` section to `CHANGELOG.md`, with `ci` running on it. One run
before it was refused creating the branch (`403 Resource not accessible by integration`) while the App was being set up;
the next, two minutes later, passed `[cause unverified]`.

The first merge, #187 (`v0.1.0`) `[verified 2026-11-15]`: the `release` job tagged the merge commit `v0.1.0` (an annotated
tag by `arustydev-thrust-release[bot]`, through the tag ruleset, with no Administration permission) and created the draft
release. `release.yml` did not start: it listened for the draft's `release: created`, and GitHub runs no workflow for the
`created`, `edited` or `deleted` events of a draft release (its documentation of the `release` event says so). So the
`release` job now starts `release.yml` itself, with `gh workflow run release.yml --ref <tag>`: a `workflow_dispatch` made
with the workflow's own token does start a run (the documented exception), and the run's ref is the tag, which the
attestations record. Its `releases` output lists every crate released, each with the tag its `git_tag_name` gives, but only
the tagger makes its tag: a crate on its first-release pattern is listed with a `<crate>-v<version>` that does not exist. So
the step takes the tagger's tag (`check-release-plz.py --release-tag`, which also requires it to be `v<version>` and the
tagger to be released once) and checks nothing else, since the tag already exists by then.

`v0.3.0`, #227 `[verified 2026-11-19]`: the step counted every crate's tag and refused six (`v0.3.0` and the five crates on
their first-release pattern), so `release.yml` never started, the release stayed a draft, and the production deploy that
ci's dispatch started stopped at `fetch-release-bundle: refused: could not read the release v0.3.0: release not found`. The
tag and the draft were right, so the recovery is `release.yml` started by hand on the tag, as its header says; the step was
changed to the rule above.

Immutable releases lock a release's assets when it is published, so release-plz makes a draft (`git_release_draft`,
read in its source: it creates the tag through the API first, then the release), and `release.yml`, dispatched on the tag,
attaches the assets and publishes it.

## Routing flags

The edge (`crates/edge`, ADR-0111) asks Flagship where a pinned request goes. Every flag of its app is made from
`.github/deploy/flagship/policy.json` by `scripts/flagship.py`; nobody edits a flag by hand.

- **The flags.**
  - `routing` is the kill switch, set from the policy's `routing`.
  - `alias-<alias>` is one flag per alias the policy lists. It holds the target the alias names, written by
    `flagship.py alias`.
  - `target-<name>` is one flag per live target. `flagship.py put` writes it and `flagship.py delete` removes it; the
    deploys and teardowns of targets call them.
  - **Who gets a target.** A target flag serves its value, `{url, deployment, release, kind, data, expires}`, only to a
    principal whose kind (`human` or `ci`) and audience (the opt-in path that admitted it) appear together in one of the
    policy's rules for that kind of target. Anyone else gets `{}`, which the edge reads as production.
- **The policy names no person.** Who may opt in through an audience is decided by the Access policy in the
  infrastructure repository; this file says which kinds and audiences reach which kind of target. `flagship.py validate`,
  run in `deps`, refuses an `@` and refuses audiences that differ from the edge's `ACCESS_AUD_<AUDIENCE>` variables.
- **Switching routing on or off** is a pull request that sets `routing` in the policy; its merge runs `sync`.
  - **In an emergency**, switching `routing` off in the dashboard, or disabling one target's flag, takes effect within
    30 s.
  - **Afterwards**, the nightly `check` reports the difference until the policy agrees or `flagship.yml` is run by hand
    to restore it.
- **`flagship.yml`.**
  - `sync` runs on a push to `main` that changes the policy, the script or the workflow, and by hand. It also deletes
    any flag the policy does not make.
  - `check --evaluate` runs after it and every night. Besides comparing definitions, it asks Flagship what each flag
    serves to every kind and audience.
  - A pull request gets only `validate` and `test_flagship.py`: it has no secrets, and a policy change is meant to
    differ from what is live.
- **To turn it on**, once OpenTofu has made the app:
  1. Create the environment `flagship`, with deployment branches limited to `main`.
  2. Give it the secrets `FLAGSHIP_WRITE_TOKEN` and `FLAGSHIP_READ_TOKEN`: app-scoped Flagship tokens, Write and Read.
  3. Give it the variables `CLOUDFLARE_ACCOUNT_ID` and `PREVIEW_HOST_SUFFIX` (the edge's).
  4. Set the repository variable `FLAGSHIP_APP_ID`.

What is not yet verified against the real service: [G-43](../testing/status.md).

## Permissions and supply chain

- All 52 external action references, of thirteen actions, are pinned to a full commit SHA with the version in a comment
  `[verified 2026-11-15 by counting the uses lines of the workflows and actions]`; `.github/scripts/lint-workflow.sh` holds
  every workflow and composite action to it, through `lint-all-workflows.sh`, which finds them by directory. `release-plz/action` pins its own actions by SHA and installs
  release-plz with `cargo-binstall` at the version the workflow names (0.4.3); release-plz publishes no attestation for its
  binaries (`gh attestation verify` finds none), so that download is trusted on its GitHub release alone. `anchore/sbom-action` (the release's SBOM) downloads
  syft, pinned to v1.54.0 in the workflow. The Rust toolchain is pinned to an exact version by `rust-toolchain.toml`, so a new clippy or
  rustfmt reaches `lint` only as a pull request: Dependabot (`.github/dependabot.yml`, the `rust-toolchain` ecosystem, weekly)
  proposes each new stable release, and a change to that file runs everything. The two `cargo metadata` calls that need no
  build (the `changes` job, `test_plan_ci.py`) use the runner's own stable (`RUSTUP_TOOLCHAIN=stable`), so they install nothing. `cargo-nextest` is 0.9.140 and `cargo-deny` is 0.20.2, both installed by
  `taiki-e/install-action`. `cloudflared` in the deploy is a release binary pinned by version and by SHA-256, and so is `actionlint` (1.7.12) in the
  `deps` job, because `taiki-e/install-action` does not offer it; the archive's build attestation from `rhysd/actionlint`
  was verified when the hash was pinned. `shellcheck` (0.11.0, the version `check-shell.sh` holds the scripts to) is installed by
  `taiki-e/install-action` in the `bash` job and in `deps`, where actionlint runs it over every workflow's `run:` blocks; the runner image's
  is 0.9.0, which does not know SC2329, and `test_workflow_wiring.py` holds the two installs to the helper's version.
- `permissions: {}` at the top of each workflow, and `contents: read` and nothing else on each job of `ci.yml` and the
  workflows it calls. `deploy.yml` asks for more, per job: `checks: read` (`plan` reads the commit's checks),
  `id-token: write` and `attestations: write` (`build` only, for the attestation), `attestations: read` and
  `actions: read` (the two deploy jobs: verify a bundle, fetch the previous release's), `actions: write` (`dispatch` only,
  to start the deploy run on the `rc/*` branch or the `v*` tag). `release.yml`: `id-token: write` and
  `attestations: write` (`build` only, for the two attestations), `contents: write` (`publish` only, which attaches the assets
  and publishes the release; nothing else in it writes). `lint-workflow.sh` enforces where
  `secrets.`, `environment:` and `id-token: write` may appear, per file as `lint-all-workflows.sh` names it; a workflow it
  does not name may have none of the three. `release-plz.yml` asks for `contents: read`, and
  `actions: write` in its `release` job to start `release.yml` on the new tag; what it
  writes, it writes with the release App's token, minted in its two jobs, the only ones `lint-workflow.sh` lets hold a
  secret there. All 22 checkouts set `persist-credentials: false`.
  Reusable-workflow and composite-action inputs reach the script through `env`, not through interpolation into the script
  text, which `lint-workflow.sh` checks in every workflow and action.
- **A workflow that other workflows call** (its `on:` has `workflow_call`; ADR-0102) is also held to two rules of
  `lint-workflow.sh`: no workflow-level `concurrency` (`github.workflow` is the caller's name there, so the group equals the
  caller's and the run deadlocks), and no `github.event_name` or `github.workflow` (both are the caller's: a callee routes by an
  input). `actionlint`, run by `lint-actionlint.sh` in the `deps` job, checks every workflow and each call in it: the `with:` it
  passes against the callee's inputs and the outputs it reads through `needs`. It checks nothing outside a git repository, so the
  script refuses to run outside one and, before it lints the repository, shows that it refuses a call that breaks the contract
  three ways (and accepts a sound one). It also reads the shell of every `run:` block, by running the shellcheck it is given
  (`-shellcheck`; with none it skips the blocks and says nothing, so the script refuses to run without one, and `deps` installs the
  pinned 0.11.0 before it): the control also shows that an unquoted variable (SC2086, so shellcheck is run at all) and a function
  that is never called, in a block that ends in `exit` (SC2329, which 0.9.0 does not know), are refused, and that a block that
  quotes and calls what it defines is accepted. actionlint runs shellcheck with `--norc`, so `.shellcheckrc` does not reach a
  workflow's shell. Its tests use stand-in binaries that each misbehave in one way, so `ci-scripts` needs no `actionlint`.
  `[verified 2026-11-23 locally, with actionlint 1.7.12 and shellcheck 0.11.0]`
- `deny.toml` configures the `deny` job (in `security.yml`): advisories, licences, bans (`sqlx`, `sea-orm`, `diesel`, `diesel-async`) and
  sources (crates.io only). Its one advisory ignore is RUSTSEC-2023-0071 (`rsa`), with the reason and when to re-check
  it written beside it; any other advisory is fixed by upgrading. The job fetches the advisory database on every run,
  so a new advisory can turn it red, and `gate` with it, with no change to the code.
- Caches are saved only on `main`. Pull-request runs restore and never save, so many cells cannot evict each other.

## Measured

PR #28's runs ended `success`, `failure` and `success`. The run on `main` after its merge ran all 24 jobs, which are
`changes`, `plan`, `ci-scripts`, `deps`, `hermetic`, `pg-adapter`, `mutants`, the 16 contract cells and `gate`, and
every one succeeded `[verified 2026-10-19 fa4bd6c]` with `gh run view`.

## Not in CI

- The `oracle_` tests, which need the pinned Plane oracle's generated output, and stage 2, which needs real
  Cloudflare resources.
- `scripts/verify-all.sh`. CI runs its steps as separate jobs, except the probes, which it does not run.
- The `blast` and `bench` contract passes, which are not in `contract-all.sh`.
- A deploy that has run against Cloudflare. `ci` tests `deploy.yml`'s scripts against a fake Cloudflare, and the pull-request
  dry run and `wrangler deploy --dry-run` check the rendered configuration. Real deploys to staging have run since 2026-11-15
  and one to production (2026-11-16; see [Deploying the Worker](deploy.md)).
- A release being published. `release-plz.yml` runs on pushes to `main` and `release.yml` on the tag, not in `ci`, and on a pull
  request `release.yml` is a dry run that attests and publishes nothing. The flow has run once: `v0.1.0` was published on
  2026-11-16 (`gh release list`), and the changelog, the workspace version, the `v*` tag ruleset and immutable releases it
  needs exist ([Release candidates](#release-candidates)).
- The upstream vitest files under `web/`, a fuzz target (there is no `fuzz/`), coverage, and `cargo mutants` (the
  `mutants` job runs the planted mutants of `scripts/mutants.sh`, on `native-memory` and `native-pg` only).

## Where the triggers are too wide or too narrow

Found by the CI/CD review of 2026-11-14 (`.agents/plans/cicd-review/`), which also holds what to do about each.

- **Too wide (fixed by PR 4, `perf/ci-affected-only`).** Before it, the `rust` flag was all or nothing: one file under
  `scripts/` (a deploy-only script included), `probes/` (which no job runs), `.github/deploy/`, a workflow that `ci`
  does not run (`flake-hunt.yml`, `e2e-schedule.yml`, `deploy.yml`) or a results file under `tools/oracle-v1/` started
  all 32 jobs (33 with `web`, which a CI file also started), and nothing narrowed by crate. Now each runs what reads it
  ([What runs when](#what-runs-when)).
- **Too narrow.** Fixed by the same PR: a change to `web/` alone now runs the `worker-cf e2e` cell (G-30), and a
  nightly run of `ci` on `main` surfaces a new advisory without waiting for a pull request.
  - Fixed: `lint-workflow.sh` checks every workflow and action (`lint-all-workflows.sh`), so `ci.yml`,
    `contract-cell.yml`, `e2e.yml`, `flake-hunt.yml` and `setup-rust` with the rest, and a new one from the day
    it is added.
  - Fixed: `hermetic` and `pg-adapter` pass `--locked`, as `lint` and `deny` did: a `Cargo.lock` that Cargo would have to
    change fails them instead of being rewritten on the runner.
  - Fixed: the toolchain is pinned by `rust-toolchain.toml`, and Dependabot proposes each new release.
  - Fixed: `flake-hunt.yml` offers every pass of the matrix (`csrfoff`, `mail`, `oracle` and `e2e` were missing), refuses
    a pairing that is not a cell, and prepares a pass with the action a cell uses.

## Against the ADRs

ADR-0100, ADR-0101 and ADR-0110 are accepted, in `.agents/plans/contrail/adrs/`. The pipeline matches them in part:

- **ADR-0100 (a build matrix per adapter set)** asks for pull requests to run lint, types and unit tests, the
  Cloudflare-set contract tests and the oracle schema check, with the full matrix and the differential oracle
  nightly. It also asks for one aggregate job as the only required context, which asserts that every expected leg
  ran, and for a periodic job that diffs the ruleset's required contexts against those reported. In place: the full
  matrix on every pull request that changes a Rust-relevant path, a lint step (`lint`: rustfmt and clippy), and the
  aggregate job, as `gate`. In place in part: the oracle schema check, a step of `deps` (`scripts/check-openapi.sh`, issue #58),
  which diffs the OpenAPI document that `thrust-api-v1` writes with the pinned Plane schema for the operations listed in
  `tools/oracle-v1/schema/described.operations` (all 32 of the scope `m1`), and runs a clippy step (`lint`) and a test step
  (`hermetic`) for the `openapi` feature that no other step turns on. Not in place: a nightly run (only the weekly flake hunt) and
  the periodic diff, which is `assert-required-contexts.sh` and is run by hand.
- **ADR-0101 (supply chain)** asks for SHA pinning, Scorecard, attestation and trivy, plus Worker-bundle
  attestation, GitHub OIDC for deploys, and Rust gates (`cargo-deny`, `cargo-vet` or `cargo audit`). In place: SHA
  pinning, one of the Rust gates, `cargo-deny` (`deny`), and the Worker bundle's attestation (SLSA provenance, which the
  deploy verifies before it uses the bundle, and verifies again on the previous release's bundle). **Deviation, by the
  owner's decision of 2026-11-13: the deploy does not use GitHub OIDC.** Cloudflare documents no OIDC federation for
  wrangler, only an API token and the account id, so each environment holds a scoped API token in its secrets, and OIDC
  stays a follow-up (recorded in ADR-0101's round-30 note). Not in place: Scorecard, trivy, and `cargo-vet` and
  `cargo audit`, which the ADR offers as alternatives.
- **ADR-0110 (the deploy pipeline)** is implemented as its round-29 update says: expand migration, `wrangler deploy` at
  100 %, the smoke, automatic rollback, with the Durable Object lifecycle rule and expand/contract discipline kept. The
  round-24 gradual rollout, with measured gates, was superseded by round 29 and is not built. What the implementation
  adds is in the round-30 note: the route to the database, where the first deploy stops, and how old web files are kept.
