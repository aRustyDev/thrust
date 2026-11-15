# CI/CD

The repository has a CI pipeline and a deploy pipeline. One workflow, `ci.yml`, runs the test suites on every pull
request. It calls a second workflow once per contract cell, a third repeats a cell on demand and weekly, and a fourth
runs the browser journeys every day. A fifth, `deploy.yml`, deploys the Worker: to staging after a push to `main`, once
the repository variable `THRUST_DEPLOY_ENABLED` is `true`, and to either environment by hand. A sixth, `release-plz.yml`,
keeps the next release's candidate pull request after every push to `main`, and tags and publishes a release when one merges. No workflow publishes a
server image. The first half of this page is how CI is built and what its one required check guarantees; [The deploy
pipeline](#the-deploy-pipeline) is how the Worker gets to an environment, and the steps to run it are on [Deploying the
Worker](deploy.md). The jobs and the command each CI job runs are in the CI section of
[Running the suites](../testing/running.md#ci), and are not repeated here.

## What exists

| File | Does |
|---|---|
| `.github/workflows/ci.yml` (`ci`) | The pipeline. Runs on pull requests, merge-queue entries, pushes to `main` and manual dispatch |
| `.github/workflows/contract-cell.yml` | A reusable workflow that runs one contract cell on its own runner |
| `.github/workflows/flake-hunt.yml` | Repeats one cell 1 to 50 times (5 by default), by hand or every Monday at 03:17 UTC. It passes only if `scripts/flake-hunt.sh` prints `failures: 0 / N`, because the script ends with an `echo`, so its exit status does not say whether an iteration failed |
| `.github/workflows/e2e-schedule.yml` | Runs the `worker-cf e2e` cell (through `contract-cell.yml`) every day at 04:41 UTC and by hand, and on a pull request that edits the file, so that the call is proved valid before it merges. It covers what the `ci` cell does not: a change to `web/` alone (`classify-changes.sh` keeps it out of `rust`) and drift in the runner, the browser and the packages. Not a gate: nothing waits for it and `gate` does not read it. The pinned-Plane leg is not here, because the oracle builds from an unpushed fork (`scripts/e2e.sh plane-oracle`, local) |
| `.github/workflows/release-plz.yml` (`release-plz`) | The release flow, run by release-plz: after each push to `main`, opens or updates the release-candidate pull request from an `rc/*` branch, and tags `vX.Y.Z` and publishes the GitHub release when such a pull request merges. See [Release candidates](#release-candidates) |
| `release-plz.toml` | release-plz's configuration: one version and one tag for the workspace, one root `CHANGELOG.md`, which commits make a release. Held to the workspace members by `.github/scripts/check-release-plz.py` |
| `.github/workflows/deploy.yml` (`deploy`) | The deploy pipeline: plan, build and attest a bundle, deploy it to an environment and prove it, roll back by itself when the smoke fails. Runs after `ci` on a push to `main`, by hand, and on a pull request that edits its files (a dry run that touches no environment). See [The deploy pipeline](#the-deploy-pipeline) |
| `.github/actions/setup-rust` | A composite action: the stable toolchain, the rustup targets and components, a keyed `rust-cache`, and optionally `worker-build` 0.8.7 and wrangler 4.151.2 |
| `.github/actions/deploy-worker` | A composite action: verify the bundle's attestation, unpack it, install wrangler (the same version as `setup-rust`, kept equal by a test) and `cloudflared` at pinned versions, and run `scripts/deploy-worker.sh` |
| `.github/scripts/` | The pipeline's own logic: `classify-changes.sh`, `plan-cells.sh`, `assert-gate.sh`, and `assert-required-contexts.sh`; and the deploy's: `plan-deploy.sh`, `classify-deploy.sh`, `active-version.py`, `assert-sha-checks.sh`, `assert-gated-environments.sh`, `verify-bundle.sh`, `unpack-bundle.sh`, `fetch-previous-assets.sh` and `lint-workflow.sh`. The `ci-scripts` job tests them with `.github/scripts/tests/` |
| `.github/deploy/` | What the deploy trusts: the Cloudflare Origin CA root that signs `thrust-pg`'s certificate, with its fingerprint pinned by a test |
| `scripts/deploy-worker.sh` and its parts | The deploy itself (render, classify, expand migration, `wrangler deploy`, smoke, rollback), run by the composite action: `render-wrangler.sh` (with the template `crates/thrust-worker/wrangler.deploy.toml`), `migrate-ci.sh`, `smoke-deploy.sh`, `smoke-bootstrap.sh`, `merge-assets.sh` and `check-asset-retention.sh` |
| `scripts/check-web-diff.sh` | The Q-R2 gate for `web/` (the stock React app, imported from the oracle pin), run by the `web` job. Tested by `.github/scripts/tests/test-check-web-diff.sh` |

## One required check

The ruleset on `main` requires exactly one status check, `gate` `[verified 2026-10-19 fa4bd6c]`. Every other job is
evidence that `gate` inspects. `gate` runs even when a job before it failed, and `.github/scripts/assert-gate.sh`
decides:

- The `changes` job must have succeeded.
- If nothing Rust-relevant changed, no job may have failed or been cancelled, and skipped jobs are expected.
- If something Rust-relevant changed, every needed job must be exactly `success`. A skipped job is a failure here,
  not a pass. The set of cells that uploaded a pass marker must also equal the planned set, which must not be empty.
- The `web` job is judged apart from those two rules, by `WEB_CHANGED` (`web/` or a CI file changed). If it is true,
  `web` must be exactly `success`, and a skipped `web` fails the gate. If it is false, `web` may be skipped, as it is on
  a Rust-only change, but not failed or cancelled; a gate run that is not told `WEB_CHANGED` fails. Eleven cases in
  `test-ci-scripts.sh` pin this `[derived: the web job has not run in CI yet]`.

The point is that "nothing failed" is not accepted as "everything ran". A cell that never started leaves no check
behind, and a required check that never reports blocks a pull request forever.
`assert-required-contexts.sh` applies the same idea from outside: it certifies a pull request by the required set of
its base branch. Nothing calls it, so it is a tool to run by hand.

## What runs when

- **Path classification.** The `changes` job sorts the changed files. The Rust jobs run only if one of them is under
  `crates/`, `scripts/`, `probes/`, `.cargo/`, `.config/`, `.github/workflows/`, `.github/actions/`,
  `.github/scripts/` or `.github/deploy/`, or is `Cargo.toml`, `Cargo.lock`, `deny.toml`, `clippy.toml`, `rustfmt.toml` or a `rust-toolchain`
  file (the last three are read by the `deny` and `lint` jobs, so a change to only one of them must still run those
  jobs `[verified 2026-11-05 7d0074f]` by the three cases in `.github/scripts/tests/test-ci-scripts.sh`). The job fails open: with no usable
  base commit, or an empty list, it runs everything. A pull request that changes only `docs/`, `AGENTS.md`,
  `.agents/` or `tools/` (but not `tools/oracle-v1/`, below) runs no tests, and `gate` passes. Run on sample lists, the
  classifier answers `rust=false` for `docs/`, `AGENTS.md`, `.agents/`, `tools/` and `.github/config/`, and `rust=true`
  for a file under `crates/`, for `.config/nextest.toml`, for a workflow file and for an empty list `[verified 2026-10-19
  fa4bd6c]`. `tools/oracle-v1/` is the v1 oracle's harness, register, corpus and results, which the three `oracle` cells
  run, so a change under it alone is `rust=true` (three cases in `test-ci-scripts.sh`, one of them a `tools/` look-alike
  that stays `rust=false`) `[verified 2026-11-10]`. On a pull
  request that touches only `docs/`, `changes` passes, `plan`, `ci-scripts`, `deps`, `hermetic`, `pg-adapter`,
  `mutants` and `contract` are skipped, and `gate` passes `[verified 2026-10-19 051b849]`. `lint` and `deny`, added
  later, have the same `if` as `deps`, so a change to only `docs/` skips them too `[derived]`. **The book is not built in
  CI at all.**
- **The web job.** `changes` also answers `web=true` when a file is under `web/`, is `scripts/check-web-diff.sh`, or is
  under `.github/workflows/`, `.github/actions/` or `.github/scripts/`, and it fails open for `web` as it does for
  `rust`. The two answers are independent: a change under `web/` alone is `rust=false` and `web=true`, so only `changes`
  and `web` run and `gate` passes on them. The `web` job runs `scripts/check-web-diff.sh` (only the files in
  `web/ALLOWED-CHANGES` may differ from the pin; `web/SOURCE` says where `web/` came from), then
  `pnpm --dir web install --frozen-lockfile --ignore-scripts` and `pnpm --dir web exec turbo run build --filter=web`,
  and checks that `web/apps/web/build/client/index.html` exists. Node is `web/.node-version`; pnpm is the version
  `web/package.json` pins, read from it. That install and build ran on the import as it is, with pnpm 11.10.0, ten
  tasks uncached from an empty `node_modules` `[verified 2026-11-11 locally; not yet in CI]`.
- **The matrix.** The `plan` job reads the `run <target> <pass>` lines of `scripts/contract-all.sh`, so no second
  list can drift. It fails if the number of cells it parsed differs from the number of lines that start with `run`,
  if there are none, or if one repeats. There are 23 cells: 20 contract cells (the browser journeys' `worker-cf e2e`
  and the password reset by email's `native-memory mail` among them) and the 3 `oracle` cells below `[verified 2026-11-14: PR #154's
  check list had 22 contract jobs, all success, before the mail cell; the 21 of PR #113 (2026-11-10) were before the e2e cell]`.
  Each runs on a fresh runner with its own checkout, which is why the shared-build-directory hazard that makes
  `contract-all.sh` run serially does not apply.
- **A cell.** `contract-cell.yml` checks that `curl`, `openssl`, `lsof`, `pgrep` and `docker` exist (and `nc` for the `mail` pass), runs
  `scripts/contract.sh <target> <pass>` with no pipe so the step's status is the script's, and uploads a marker named
  for the cell. The script's target guard and count guard apply as they do locally.
- **The mail cell.** `native-memory mail` starts a Mailpit container (pinned, pulled from Docker Hub as `postgres` is) beside the native server, which is
  given `THRUST_SMTP_*` and `THRUST_PUBLIC_BASE_URL`, and runs `tests/mail.rs`: the password reset by email through a mail server written by someone else.
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
| `plan` | no secret | Which event and which commit (`plan-deploy.sh`, tested over every combination of its inputs). A push to `main` deploys staging after `ci` passed, and only while `THRUST_DEPLOY_ENABLED` is `true`. A manual run names `staging` or `production` and a commit of `main`, and must itself run from `main`. A pull request is a dry run. The commit's own `gate` must have passed, asked for by name on that commit (`assert-sha-checks.sh`), and the environments that gate the deploy must have required reviewers (`assert-gated-environments.sh`: GitHub creates an unprotected environment when a job names one that does not exist) |
| `build` | no secret | The Worker, the `migrate` binary and the web app (`web/`), into one bundle, attested (SLSA provenance) in a deploy |
| `dry-run` | none | For a pull request that edits the deploy files: the bundle is built and `wrangler deploy --dry-run` checks the rendered configuration. No environment, no secret |
| `deploy` | the environment's secrets | Verify the attestation, render the configuration, read what is live, classify the change, run the expand migration only if migration files changed, keep the previous release's web files, `wrangler deploy --tag <sha>`, the smoke, and its controls. It defers, deploying nothing, whatever cannot be rolled back across |
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

**Not yet run for real.** No environment has had a deploy through it. The shapes of `wrangler deployments list --json` and
`versions list --json` are the documented ones (the parser sends any other shape to the manual gate, never to an automatic
rollback), and the token's scopes, the queue consumer with a dead-letter queue, and the attestation flow end to end have
not run. The pull-request dry run and the first two `workflow_run` events have `[verified 2026-11-14]`: on PR #165's run
`plan`, `build` and `dry-run` succeeded and wrangler 4.151.2 accepted the rendered template against the bundle CI built; on
`main`, the run after a failed `ci` planned `skip` ("ci did not succeed") and the run after a green one planned `skip`
("automatic deploys are off").

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
  tags the merge commit `vX.Y.Z` and publishes the GitHub release with that version's changelog section. Any other push is
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

How each setting was checked: on a copy of this workspace with release-plz 0.4.3 `[verified 2026-11-15]`. Right after a
`v0.1.0` tag nothing is proposed; a fix in one crate proposes 0.1.1, rewrites `Cargo.toml`'s one version line, keeps
`Cargo.lock` consistent (`cargo metadata --locked` passes) and lists only that fix; a feature then proposes 0.2.0; a
breaking `chore!:` proposes 0.2.0; test and docs commits alone propose nothing; a fix only under `web/` proposes nothing.
Two settings the docs do not settle were read in the source instead: `release_always = false` matches the configured
`pr_branch_prefix` (the docs still name `release-plz-`), and a tag pattern set only on `thrust-core` made every other crate
look for a tag of its own and re-release all of history, which is why `git_tag_name` is set for the whole workspace. The
`release-pr` and `release` jobs themselves have not run yet: they need the App.

**Not live yet.** Until the repository variable `RELEASE_APP_CLIENT_ID` is set, both jobs are skipped. To turn it on, the
owner:

1. creates a GitHub App (no webhook) with repository permissions Contents: read and write, Pull requests: read and write,
   and installs it on this repository only;
2. creates the environment `release`, with deployment branches limited to `main`, and the secret `RELEASE_APP_PRIVATE_KEY`
   (the App's private key) in it;
3. sets the repository variable `RELEASE_APP_CLIENT_ID` to the App's client id (after step 2: GitHub creates an
   unprotected environment when a job names one that does not exist);
4. lets only the App and admins create, update or delete `rc/*` (a ruleset with the App as a bypass actor), and protects
   `v*` tags the same way. If a tag ruleset restricts who may create tags, the App also needs Administration: read and
   write, which release-plz documents for protected tags.

The attestation of a release and the production deploy from its tag are the next pull request
(`.agents/plans/cicd-review/PLAN.md`, PR 3).

## Permissions and supply chain

- All 44 external action references, of eleven actions, are pinned to a full commit SHA with the version in a comment
  `[verified 2026-11-15 by counting the uses lines of the workflows and actions]`; `.github/scripts/lint-workflow.sh` holds
  `deploy.yml`, its action and `release-plz.yml` to it. `release-plz/action` pins its own actions by SHA and installs
  release-plz with `cargo-binstall` at the version the workflow names (0.4.3); release-plz publishes no attestation for its
  binaries (`gh attestation verify` finds none), so that download is trusted on its GitHub release alone. The Rust toolchain is `stable`, not pinned, so a new clippy or rustfmt release can fail `lint`
  with no change to the code. `cargo-nextest` is 0.9.140 and `cargo-deny` is 0.20.2, both installed by
  `taiki-e/install-action`. `cloudflared` in the deploy is a release binary pinned by version and by SHA-256.
- `permissions: {}` at the top of each workflow, and `contents: read` and nothing else on each job of `ci.yml` and the
  workflows it calls. `deploy.yml` asks for more, per job: `checks: read` (`plan` reads the commit's checks),
  `id-token: write` and `attestations: write` (`build` only, for the attestation), `attestations: read` and
  `actions: read` (the two deploy jobs: verify a bundle, fetch the previous release's). `lint-workflow.sh` enforces where
  `secrets.`, `environment:` and `id-token: write` may appear. `release-plz.yml` asks for `contents: read` only; what it
  writes, it writes with the release App's token, minted in its two jobs, the only ones `lint-workflow.sh` lets hold a
  secret there. All 20 checkouts set `persist-credentials: false`.
  Reusable-workflow and composite-action inputs reach the script through `env`, not through interpolation into the script
  text, which `lint-workflow.sh` checks for `deploy.yml` and its action.
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
- The book build and its checks.
- A deploy that has run against Cloudflare. `deploy.yml` is tested against a fake Cloudflare, by its pull-request dry run
  and by `wrangler deploy --dry-run`; no environment has had a deploy through it yet (see [Deploying the Worker](deploy.md)).
- A release. There is no changelog, version (the workspace is `0.0.0`), release tag, GitHub release or tag protection.
- The upstream vitest files under `web/`, a fuzz target (there is no `fuzz/`), coverage, and `cargo mutants` (the
  `mutants` job runs the planted mutants of `scripts/mutants.sh`, on `native-memory` and `native-pg` only).

## Where the triggers are too wide or too narrow

Found by the CI/CD review of 2026-11-14 (`.agents/plans/cicd-review/`), which also holds what to do about each.

- **Too wide.** `rust=true` is all or nothing: one file under `scripts/` (a deploy-only script included), `probes/`
  (which no job runs), `.github/deploy/`, a workflow that `ci` does not run (`flake-hunt.yml`, `e2e-schedule.yml`,
  `deploy.yml`) or a results file under `tools/oracle-v1/` starts all 32 jobs (33 with `web`, which a CI file also starts): the 22
  cells, `mutants`, `pg-adapter` and the rest. There is no narrowing by crate.
- **Too narrow.** A change to `web/` alone does not run the `worker-cf e2e` cell, which serves that build (G-30).
  `lint-workflow.sh` checks only `deploy.yml` and its action, not `ci.yml`, `contract-cell.yml`, `e2e-schedule.yml`,
  `flake-hunt.yml` or `setup-rust`. `hermetic` and `pg-adapter` do not pass `--locked`. The toolchain floats on `stable`
  and the advisory database is fetched live, and with no scheduled run of `ci` either surfaces only on the next
  Rust-relevant pull request. `flake-hunt.yml` cannot select the `csrfoff`, `oracle` or `e2e` passes.

## Against the ADRs

ADR-0100, ADR-0101 and ADR-0110 are accepted, in `.agents/plans/contrail/adrs/`. The pipeline matches them in part:

- **ADR-0100 (a build matrix per adapter set)** asks for pull requests to run lint, types and unit tests, the
  Cloudflare-set contract tests and the oracle schema check, with the full matrix and the differential oracle
  nightly. It also asks for one aggregate job as the only required context, which asserts that every expected leg
  ran, and for a periodic job that diffs the ruleset's required contexts against those reported. In place: the full
  matrix on every pull request that changes a Rust-relevant path, a lint step (`lint`: rustfmt and clippy), and the
  aggregate job, as `gate`. In place in part: the oracle schema check, a step of `deps` (`scripts/check-openapi.sh`, issue #58),
  which diffs the OpenAPI document that `thrust-api-v1` writes with the pinned Plane schema for the operations listed in
  `tools/oracle-v1/schema/described.operations` (the 8 Projects ones so far), and runs a clippy step (`lint`) and a test step
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
