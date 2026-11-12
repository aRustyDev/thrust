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
| `.github/workflows/e2e-schedule.yml` | Runs the `worker-cf e2e` cell (through `contract-cell.yml`) every day at 04:41 UTC and by hand, and on a pull request that edits the file, so that the call is proved valid before it merges. It covers what the `ci` cell does not: a change to `web/` alone (`classify-changes.sh` keeps it out of `rust`) and drift in the runner, the browser and the packages. Not a gate: nothing waits for it and `gate` does not read it. The pinned-Plane leg is not here, because the oracle builds from an unpushed fork (`scripts/e2e.sh plane-oracle`, local) |
| `.github/actions/setup-rust` | A composite action: the stable toolchain, the rustup targets and components, a keyed `rust-cache`, and optionally `worker-build` 0.8.7 and wrangler 4.151.2 |
| `.github/scripts/` | The pipeline's own logic: `classify-changes.sh`, `plan-cells.sh`, `assert-gate.sh`, and `assert-required-contexts.sh`. The `ci-scripts` job tests them with `.github/scripts/tests/` |
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
  `crates/`, `scripts/`, `probes/`, `.cargo/`, `.config/`, `.github/workflows/`, `.github/actions/` or
  `.github/scripts/`, or is `Cargo.toml`, `Cargo.lock`, `deny.toml`, `clippy.toml`, `rustfmt.toml` or a `rust-toolchain`
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
  if there are none, or if one repeats. There are 21 cells: 18 contract cells and the 3 `oracle` cells below `[verified 2026-11-10 43a17da: PR #113's run had 21 cell jobs, all succeeded, and `gate` printed `ok: 21 of 21 planned cells reported success`; on the run before (e591b29) `hermetic` had failed, so the script stopped at the job check and that comparison did not run]`.
  Each runs on a fresh runner with its own checkout, which is why the shared-build-directory hazard that makes
  `contract-all.sh` run serially does not apply.
- **A cell.** `contract-cell.yml` checks that `curl`, `openssl`, `lsof`, `pgrep` and `docker` exist, runs
  `scripts/contract.sh <target> <pass>` with no pipe so the step's status is the script's, and uploads a marker named
  for the cell. The script's target guard and count guard apply as they do locally.
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
  never cancelled.

## Permissions and supply chain

- All 21 external action references are pinned to a full commit SHA with the version in a comment `[verified
  2026-11-05 7d0074f]`; `astral-sh/setup-uv` (v10.3.2, 19 days old when pinned) is the 22nd. The Rust toolchain is `stable`, not pinned, so a new clippy or rustfmt release can fail `lint`
  with no change to the code. `cargo-nextest` is 0.9.140 and `cargo-deny` is 0.20.2, both installed by
  `taiki-e/install-action`.
- `permissions: {}` at the top of each workflow, and `contents: read` and nothing else on each job. All 12 checkouts
  set `persist-credentials: false`. Reusable-workflow inputs reach the script through `env`, not through
  interpolation into the script text.
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
- Any deploy. There is no workflow for it.

## Against the ADRs

ADR-0100, ADR-0101 and ADR-0110 are accepted, in `.agents/plans/contrail/adrs/`. The pipeline matches them in part:

- **ADR-0100 (a build matrix per adapter set)** asks for pull requests to run lint, types and unit tests, the
  Cloudflare-set contract tests and the oracle schema check, with the full matrix and the differential oracle
  nightly. It also asks for one aggregate job as the only required context, which asserts that every expected leg
  ran, and for a periodic job that diffs the ruleset's required contexts against those reported. In place: the full
  matrix on every pull request that changes a Rust-relevant path, a lint step (`lint`: rustfmt and clippy), and the
  aggregate job, as `gate`. Not in place: a nightly run (only the weekly flake hunt), the oracle schema check and the
  periodic diff, which is `assert-required-contexts.sh` and is run by hand.
- **ADR-0101 (supply chain)** asks for SHA pinning, Scorecard, attestation and trivy, plus Worker-bundle
  attestation, GitHub OIDC for deploys, and Rust gates (`cargo-deny`, `cargo-vet` or `cargo audit`). In place: SHA
  pinning, and one of the Rust gates, `cargo-deny` (`deny`). Not in place: the rest, including `cargo-vet` and
  `cargo audit`, which the ADR offers as alternatives. Of those, OIDC and bundle attestation depend on a deploy, and
  the others do not.
- **ADR-0110 (the deploy pipeline)** describes a gradual rollout with measured gates and automatic rollback. Nothing
  implements it.
