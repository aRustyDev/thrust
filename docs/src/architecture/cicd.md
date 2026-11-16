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
| `.github/workflows/contract-cell.yml` | A reusable workflow that runs one contract cell on its own runner |
| `.github/workflows/flake-hunt.yml` | Repeats one cell 1 to 50 times (5 by default), by hand or every Monday at 03:17 UTC. It passes only if `scripts/flake-hunt.sh` prints `failures: 0 / N`, because the script ends with an `echo`, so its exit status does not say whether an iteration failed |
| `.github/workflows/e2e-schedule.yml` | Runs the `worker-cf e2e` cell (through `contract-cell.yml`) every day at 04:41 UTC and by hand, and on a pull request that edits the file, so that the call is proved valid before it merges. It covers what a pull request does not: drift in the runner, the browser and the packages, every day on `main`. Not a gate: nothing waits for it and `gate` does not read it. The pinned-Plane leg is not here, because the oracle builds from an unpushed fork (`scripts/e2e.sh plane-oracle`, local) |
| `.github/workflows/release-plz.yml` (`release-plz`) | The release flow, run by release-plz: after each push to `main`, opens or updates the release-candidate pull request from an `rc/*` branch, and tags `vX.Y.Z`, creates the GitHub release as a draft and starts `release.yml` on the tag when such a pull request merges. See [Release candidates](#release-candidates) |
| `.github/workflows/release.yml` (`release`) | Dispatched on the release tag by `release-plz.yml` (a draft release starts no workflow), or by hand on the tag; which run releases what is `plan-release.sh`. It builds the bundle at the tag, attests its build provenance and an SPDX SBOM of the source, attaches the bundle, the SBOM and `SHA256SUMS`, and publishes the release (immutable). On a pull request that edits it or the build, a dry run that attests and publishes nothing. See [Releasing](release.md) |
| `release-plz.toml` | release-plz's configuration: one version and one tag for the workspace, one root `CHANGELOG.md`, which commits make a release. Held to the workspace members by `.github/scripts/check-release-plz.py` |
| `.github/workflows/deploy.yml` (`deploy`) | The deploy pipeline: plan, build and attest a release candidate's bundle (staging) or take a release's (production), deploy it to an environment and prove it, roll back by itself when the smoke fails. Runs after `ci` on a release candidate or a tagged push to `main`, by hand, and on a pull request that edits its files (a dry run that touches no environment). See [The deploy pipeline](#the-deploy-pipeline) |
| `.github/actions/setup-rust` | A composite action: the Rust version `rust-toolchain.toml` pins (read by `.github/scripts/toolchain-channel.sh`, which refuses anything but an exact version), the rustup targets and components, a keyed `rust-cache`, and optionally `worker-build` 0.8.7 and wrangler 4.151.2 |
| `.github/actions/build-bundle` | A composite action: the deploy bundle of the checked-out commit (the Worker, the `migrate` binary and the web app, in one reproducible file), used by `deploy.yml` and `release.yml` so that both build it the same way |
| `.github/actions/deploy-worker` | A composite action: fetch the bundle (this run's build for staging, the release's asset for production, `fetch-release-bundle.sh`), verify its attestation, unpack it, install wrangler (the same version as `setup-rust`, kept equal by a test) and `cloudflared` at pinned versions, and run `scripts/deploy-worker.sh` |
| `.github/scripts/` | The pipeline's own logic: `plan-ci.py`, `plan-cells.sh`, `assert-gate.sh`, `assert-required-contexts.sh` and `lint-all-workflows.sh`; and the deploy's: `plan-deploy.sh`, `classify-deploy.sh`, `active-version.py`, `assert-sha-checks.sh`, `assert-gated-environments.sh`, `verify-bundle.sh`, `unpack-bundle.sh`, `fetch-previous-assets.sh`, `fetch-release-bundle.sh` and `lint-workflow.sh`; and the release's: `plan-release.sh`. The `ci-scripts` job tests them with `.github/scripts/tests/` |
| `.github/deploy/` | What the deploy trusts: the Cloudflare Origin CA root that signs `thrust-pg`'s certificate, with its fingerprint pinned by a test |
| `scripts/deploy-worker.sh` and its parts | The deploy itself (render, classify, expand migration, `wrangler deploy`, smoke, rollback), run by the composite action: `render-wrangler.sh` (with the template `crates/thrust-worker/wrangler.deploy.toml`), `migrate-ci.sh`, `smoke-deploy.sh`, `smoke-bootstrap.sh`, `merge-assets.sh` and `check-asset-retention.sh` |
| `scripts/check-web-diff.sh` | The Q-R2 gate for `web/` (the stock React app, imported from the oracle pin), run by the `web` job. Tested by `.github/scripts/tests/test-check-web-diff.sh` |

## One required check

The ruleset on `main` requires exactly one status check, `gate` `[verified 2026-10-19 fa4bd6c]`. Every other job is
evidence that `gate` inspects. `gate` runs even when a job before it failed, and `.github/scripts/assert-gate.sh`
decides:

- The `changes` job must have succeeded. It names the jobs that must run (`jobs`) and the contract cells (`cells`), from
  `.github/scripts/plan-ci.py` (see [What runs when](#what-runs-when)).
- Every expected job must be exactly `success`. A skipped job is a failure here, not a pass.
- Every job that is not expected must be `skipped`. One that ran anyway means its `if:` and the planner disagree.
- The planner and the gate list the same ten jobs. An expected job that is not a need of the gate, or a need the
  planner does not know, fails it, as does an input that is missing or is not a JSON array of strings.
- `contract` is expected exactly when cells are planned. The set of cells that uploaded a pass marker must then equal
  the planned set, which must not be empty.
- `test-ci-scripts.sh` generates the cases: each of the ten jobs, expected or not, with each of the four results (80
  cases), then the cells, the inputs and the names. 34 hand mutants of the planner and the gate are each caught by
  their tests `[verified 2026-11-16 locally, under dash; not yet in CI]`.

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
    affected or the suite changed.
  - Some files outside the crates are read by the crates' tests, which Cargo's graph cannot see, so they run those tests'
    jobs:
    - `tools/oracle-v1/` (its schema and corpus are read by the tests of four crates) runs `hermetic`, `deps` (the
      OpenAPI diff) and the three `oracle` cells.
    - `.github/deploy/` (read by the TLS tests of `thrust-adapters-postgres`) runs `hermetic` and `ci-scripts`.
  - A file that only one pass reads runs that pass on every target:
    - `e2e/`, `scripts/e2e.sh` and the e2e tools run `worker-cf e2e`.
    - `web/` runs the `web` job and `worker-cf e2e`. A change to `web/` alone is now driven in a browser on its own pull
      request (G-30's gap 2).
  - A file that only a guard or a job reads runs that job:
    - `deny.toml` runs `deny`; `clippy.toml` and `rustfmt.toml` run `lint`.
    - `release-plz.toml` and the guards of `deps` run `deps`.
    - A deploy script (`smoke-deploy.sh`, `migrate-ci.sh`, …) runs only `ci-scripts`.
    - Another workflow, action or helper script runs `ci-scripts` and `deps` (the workflow lint).
  - Everything runs on a path whose readers the planner does not know:
    - the workspace manifest, the lockfile and the toolchain;
    - `.cargo/` and `.config/`;
    - `ci.yml`, `contract-cell.yml` and `setup-rust`;
    - the planner and the gate;
    - the scripts that run a cell;
    - an unknown directory under `crates/`;
    - an unlisted script under `scripts/`.
  - The book's inputs run the `docs` job: `docs/`, `AGENTS.md` (the book includes its testing rules), `.config/branding/`
    (its palette) and `scripts/check-book.sh`.
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
- **Examples** (the planner run on this repository) `[verified 2026-11-16 locally]`:

  | Change | Jobs | Cells |
  |---|---|---|
  | a file under `web/` | `web`, `contract` | 1 (`worker-cf e2e`) |
  | `thrust-adapters-cloudflare` | `ci-scripts`, `deps`, `lint`, `hermetic`, `contract` | the 13 `worker-*` cells |
  | `thrust-adapters-selfhosted/Cargo.toml` | the same, plus `deny`, `pg-adapter` and `mutants` | the 10 `native-*` cells |
  | `thrust-contracts` (everything depends on it) | all but `deny` and `web` | all 23 |
  | `tools/oracle-v1/` | `deps`, `hermetic`, `contract` | the 3 `oracle` cells |
  | `scripts/smoke-deploy.sh` | `ci-scripts` | none |
  | a page under `docs/` | `docs` | none |

- **Tested** by `test_plan_ci.py`, about 4,900 checks:
  - 300 generated crate graphs, with each crate changed alone and in pairs, against an independent reverse-reachability;
  - the plan of a list equals the union of the plans of its parts;
  - every rule is the first match of its own sample, and matches a tracked file;
  - the fail-open cases;
  - this repository's real graph and targets;
  - no script that the planner keeps out of the cells is called by what runs a cell.

  `[verified 2026-11-16 locally; not yet in CI]`
- **The web job.** The `web` job runs `scripts/check-web-diff.sh` (only the files in
  `web/ALLOWED-CHANGES` may differ from the pin; `web/SOURCE` says where `web/` came from), then
  `pnpm --dir web install --frozen-lockfile --ignore-scripts` and `pnpm --dir web exec turbo run build --filter=web`,
  and checks that `web/apps/web/build/client/index.html` exists. Node is `web/.node-version`; pnpm is the version
  `web/package.json` pins, read from it. That install and build ran on the import as it is, with pnpm 11.10.0, ten
  tasks uncached from an empty `node_modules` `[verified 2026-11-11 locally; not yet in CI]`.
- **The matrix.** The `changes` job reads the `run <target> <pass>` lines of `scripts/contract-all.sh`
  (`plan-cells.sh`), so no second list can drift, and the planner keeps the cells the change needs. It fails if the number of cells it parsed differs from the number of lines that start with `run`,
  if there are none, or if one repeats. There are 25 cells: 22 contract cells (the browser journeys' `worker-cf e2e`
  and the password reset by email's three `mail` cells, `native-memory`, `worker-memory` and `worker-cf`, among them) and the 3 `oracle` cells below `[verified 2026-11-14: PR #154's
  check list had 22 contract jobs, all success, before the first mail cell; the 21 of PR #113 (2026-11-10) were before the e2e cell]`.
  Each runs on a fresh runner with its own checkout, which is why the shared-build-directory hazard that makes
  `contract-all.sh` run serially does not apply.
- **A cell.** `contract-cell.yml` checks that `curl`, `openssl`, `lsof`, `pgrep` and `docker` exist (and `nc` for the native `mail` cell, which waits for its Mailpit with it), runs
  `scripts/contract.sh <target> <pass>` with no pipe so the step's status is the script's, and uploads a marker named
  for the cell. The script's target guard and count guard apply as they do locally.
- **The mail cells.** `native-memory mail` starts a Mailpit container (pinned, pulled from Docker Hub as `postgres` is) beside the native server, which is
  given `THRUST_SMTP_*` and `THRUST_PUBLIC_BASE_URL`, and runs `tests/mail.rs`: the password reset by email through a mail server written by someone else.
  `worker-memory mail` and `worker-cf mail` (E3) run the same three tests against the Worker built for wasm32 and run by `wrangler dev`, whose
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
| `build` | no secret | Staging and a dry run only: the Worker, the `migrate` binary and the web app (`web/`), into one bundle (`.github/actions/build-bundle`), attested (SLSA provenance) in a deploy. Production builds nothing: its bundle is the release's, built once at the tag by `release.yml` |
| `dry-run` | none | For a pull request that edits the deploy files: the bundle is built and `wrangler deploy --dry-run` checks the rendered configuration. No environment, no secret |
| `deploy` | the environment's secrets | Fetch the bundle: this run's artifact for staging; for production the release's asset, only from a published, immutable release and only the file GitHub's release attestation names (`fetch-release-bundle.sh`). Verify its attestation (signed on exactly this run's ref by `deploy.yml` for an `rc/*` branch, by `release.yml` for a `v*` tag; `verify-bundle.sh` refuses any other ref without asking), render the configuration, read what is live, classify the change, run the expand migration only if migration files changed, keep the previous release's web files, `wrangler deploy --tag <sha>`, the smoke, and its controls. It defers, deploying nothing, whatever cannot be rolled back across |
| `deploy-do` | the same secrets, behind reviewers | What `deploy` deferred: the first deploy to an environment, any change to `[[migrations]]`, a live state that cannot be read, a previous release's web files that cannot be found. No automatic rollback |

The controls, each with a test that makes it fail:

- **The smoke and its control.** `scripts/smoke-deploy.sh` needs the version that answers (`x-thrust-version`) to be the commit
  just deployed, and a real read to work. The same smoke is then asked for a version nobody deployed and must refuse it: a
  smoke that passes that cannot tell one version from another, and the run fails on it.
- **Rollback.** When either fails and a rollback is possible, `wrangler rollback <previous version>` runs and the
  unauthenticated probe must show the previous version answering. The run stays red either way, and the database keeps the
  expanded schema (expand/contract: the previous code runs on it).
- **The manual gate.** The first deploy carries `[[migrations]] v1`, and a Durable Object lifecycle change can be neither
  gradual nor rolled back across. They go through `deploy-do`.
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
**Not yet run:** production, and a narrower token than the one in use. **A known gap:** release-plz force-updates the release
candidate's branch, so the live commit is usually not in the next candidate's history, and the classifier, which fails closed,
sends that deploy to the manual gate although nothing in it needs one (runs 43796746005 and 43803593395).

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
attestations record. Its `releases` output lists each crate under the one workspace tag; the step refuses anything but one
tag.

Immutable releases lock a release's assets when it is published, so release-plz makes a draft (`git_release_draft`,
read in its source: it creates the tag through the API first, then the release), and `release.yml`, dispatched on the tag,
attaches the assets and publishes it.

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
  `taiki-e/install-action`. `cloudflared` in the deploy is a release binary pinned by version and by SHA-256.
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
- `deny.toml` configures the `deny` job: advisories, licences, bans (`sqlx`, `sea-orm`, `diesel`, `diesel-async`) and
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
  and none to production (see [Deploying the Worker](deploy.md)).
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
    `contract-cell.yml`, `e2e-schedule.yml`, `flake-hunt.yml` and `setup-rust` with the rest, and a new one from the day
    it is added.
  - Fixed: `hermetic` and `pg-adapter` pass `--locked`, as `lint` and `deny` did: a `Cargo.lock` that Cargo would have to
    change fails them instead of being rewritten on the runner.
  - Fixed: the toolchain is pinned by `rust-toolchain.toml`, and Dependabot proposes each new release.
  - Still open: `flake-hunt.yml` cannot select the `csrfoff`, `oracle` or `e2e` passes.

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
