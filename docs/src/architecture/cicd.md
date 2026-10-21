# CI/CD

The repository has a CI pipeline and no CD. One workflow, `ci.yml`, runs the test suites on every pull request. It
calls a second workflow once per contract cell, and a third repeats a cell on demand and weekly. Nothing deploys: no
workflow publishes a Worker or a server image. This page is about how the pipeline is built and what its one
required check guarantees. The jobs and the command each runs are in the CI section of
[Running the suites](../testing/running.md#ci), and are not repeated here.

## What exists

| File | Does |
|---|---|
| `.github/workflows/ci.yml` (`ci`) | The pipeline. Runs on pull requests, merge-queue entries, pushes to `main` and manual dispatch |
| `.github/workflows/contract-cell.yml` | A reusable workflow that runs one contract cell on its own runner |
| `.github/workflows/flake-hunt.yml` | Repeats one cell 1 to 50 times (5 by default), by hand or every Monday at 03:17 UTC. It passes only if `scripts/flake-hunt.sh` prints `failures: 0 / N`, because the script ends with an `echo`, so its exit status does not say whether an iteration failed |
| `.github/actions/setup-rust` | A composite action: the stable toolchain, the rustup targets, a keyed `rust-cache`, and optionally `worker-build` 0.8.7 and wrangler 4.151.2 |
| `.github/scripts/` | The pipeline's own logic: `classify-changes.sh`, `plan-cells.sh`, `assert-gate.sh`, and `assert-required-contexts.sh`. The `ci-scripts` job tests them with `.github/scripts/tests/` |

## One required check

The ruleset on `main` requires exactly one status check, `gate` `[verified 2026-10-19 fa4bd6c]`. Every other job is
evidence that `gate` inspects. `gate` runs even when a job before it failed, and `.github/scripts/assert-gate.sh`
decides:

- The `changes` job must have succeeded.
- If nothing Rust-relevant changed, no job may have failed or been cancelled, and skipped jobs are expected.
- If something Rust-relevant changed, every needed job must be exactly `success`. A skipped job is a failure here,
  not a pass. The set of cells that uploaded a pass marker must also equal the planned set, which must not be empty.

The point is that "nothing failed" is not accepted as "everything ran". A cell that never started leaves no check
behind, and a required check that never reports blocks a pull request forever.
`assert-required-contexts.sh` applies the same idea from outside: it certifies a pull request by the required set of
its base branch. Nothing calls it, so it is a tool to run by hand.

## What runs when

- **Path classification.** The `changes` job sorts the changed files. The Rust jobs run only if one of them is under
  `crates/`, `scripts/`, `probes/`, `.cargo/`, `.config/`, `.github/workflows/`, `.github/actions/` or
  `.github/scripts/`, or is `Cargo.toml`, `Cargo.lock` or a `rust-toolchain` file. The job fails open: with no usable
  base commit, or an empty list, it runs everything. A pull request that changes only `docs/`, `AGENTS.md`,
  `.agents/` or `tools/` runs no tests, and `gate` passes. Run on sample lists, the classifier answers `rust=false`
  for `docs/`, `AGENTS.md`, `.agents/`, `tools/` and `.github/config/`, and `rust=true` for a file under `crates/`,
  for `.config/nextest.toml`, for a workflow file and for an empty list `[verified 2026-10-19 fa4bd6c]`. On a pull
  request that touches only `docs/`, `changes` passes, `plan`, `ci-scripts`, `deps`, `hermetic`, `pg-adapter`,
  `mutants` and `contract` are skipped, and `gate` passes `[verified 2026-10-19 051b849]`. **The book is not built in
  CI at all.**
- **The matrix.** The `plan` job reads the `run <target> <pass>` lines of `scripts/contract-all.sh`, so no second
  list can drift. It fails if the number of cells it parsed differs from the number of lines that start with `run`,
  if there are none, or if one repeats. There are 16 cells. Each runs on a fresh runner with its own checkout, which
  is why the shared-build-directory hazard that makes `contract-all.sh` run serially does not apply.
- **A cell.** `contract-cell.yml` checks that `curl`, `openssl`, `lsof`, `pgrep` and `docker` exist, runs
  `scripts/contract.sh <target> <pass>` with no pipe so the step's status is the script's, and uploads a marker named
  for the cell. The script's target guard and count guard apply as they do locally.
- **Concurrency.** A new push cancels the older run of a pull request. Runs on `main` and in the merge queue are
  never cancelled.

## Permissions and supply chain

- All 18 external action references are pinned to a full commit SHA with the version in a comment `[verified
  2026-10-19 fa4bd6c]`. The Rust toolchain is `stable`, not pinned. `cargo-nextest` is 0.9.140, installed by
  `taiki-e/install-action`.
- `permissions: {}` at the top of each workflow, and `contents: read` and nothing else on each job. All 10 checkouts
  set `persist-credentials: false`. Reusable-workflow inputs reach the script through `env`, not through
  interpolation into the script text.
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
- The book build and its checks, `cargo fmt`, `cargo clippy` and any licence or dependency audit.
- Any deploy. There is no workflow for it.

## Against the ADRs

ADR-0100, ADR-0101 and ADR-0110 are accepted, in `.agents/plans/contrail/adrs/`. The pipeline matches them in part:

- **ADR-0100 (a build matrix per adapter set)** asks for pull requests to run lint, types and unit tests, the
  Cloudflare-set contract tests and the oracle schema check, with the full matrix and the differential oracle
  nightly. It also asks for one aggregate job as the only required context, which asserts that every expected leg
  ran, and for a periodic job that diffs the ruleset's required contexts against those reported. In place: the full
  matrix on every pull request that changes a Rust-relevant path, and the aggregate job, as `gate`. Not in place: a
  lint step, a nightly run (only the weekly flake hunt), the oracle schema check and the periodic diff, which is
  `assert-required-contexts.sh` and is run by hand.
- **ADR-0101 (supply chain)** asks for SHA pinning, Scorecard, attestation and trivy, plus Worker-bundle
  attestation, GitHub OIDC for deploys, and Rust gates (`cargo-deny`, `cargo-vet` or `cargo audit`). In place: SHA
  pinning. Not in place: the rest. Of those, OIDC and bundle attestation depend on a deploy, and the others do not.
- **ADR-0110 (the deploy pipeline)** describes a gradual rollout with measured gates and automatic rollback. Nothing
  implements it.
