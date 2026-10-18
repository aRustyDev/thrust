# Worked example: the contract matrix as a CI pipeline

The repo's own matrix is the model: **one HTTP contract suite, run unchanged against every target**
(`native-memory`, `native-pg`, `worker-memory`, `worker-cf`), each in several passes. The cell list is the
`run <target> <pass>` lines of `scripts/contract-all.sh`; expected test counts and the vacuity guards live in
`scripts/contract.sh`. Neither is restated here: read them. What the contract tests assert, and how to write or
mutation-check them, belongs to `rust-testing`; the Worker crates, `wrangler` and `worker-build` belong to
`rust-on-workers`.

## Preconditions (assumed contract)

The example assumes these repo files exist and behave as the scripts' own headers describe; re-read them
before adopting the pipeline, and re-check every path:

- `scripts/contract.sh <target> [pass]`: exits with the status of `cargo test` itself and refuses to pass when
  the passed count differs from the expected one or when the target reports a different adapter set than the
  cell claims.
- `scripts/contract-all.sh`: one `run <target> <pass>` line per cell, the single source the CI cell list is derived from.
- `scripts/flake-hunt.sh <target> <pass> <n>`, `scripts/check-deps.sh`, `scripts/mutants.sh`, `scripts/rs`,
  `scripts/worker-dev.sh`, `crates/thrust-worker/wrangler.toml`, and the workspace `[profile.release]`.

Tags: `[derived]` = inferred, not executed; `[unverified: not run]` = not executed. The pipeline files were
linted and their helper scripts tested locally; **no workflow was executed on GitHub**.

## File map (`examples/github/`, copy to `.github/`)

| File | Seam | Role |
|---|---|---|
| `workflows/ci.yml` | entry workflow | `changes` -> `plan` -> (`ci-scripts`, `deps`, `contract` matrix, `mutants`) -> `gate` |
| `workflows/contract-cell.yml` | reusable workflow | one cell on its own runner; uploads a pass marker and logs |
| `workflows/flake-hunt.yml` | entry workflow | `workflow_dispatch` inputs + weekly `schedule`; repeats one cell |
| `actions/setup-rust/action.yml` | composite action | toolchain, targets, keyed cache, pinned `worker-build` + `wrangler` |
| `scripts/classify-changes.sh` | script | paths on stdin -> `rust=true|false`, fails open |
| `scripts/plan-cells.sh` | script | derive matrix + planned set from `scripts/contract-all.sh` |
| `scripts/assert-gate.sh` | script | the `gate` logic: planned set must equal reported set |
| `scripts/assert-required-contexts.sh` | script | certify a PR from outside CI |
| `scripts/tests/*.sh` | tests | gate, plan, classifier and certifier cases incl. must-fail controls; no GitHub needed; run by the `ci-scripts` job |

Local checks of these files (2026-10-18): `actionlint` 1.7.12 in a git checkout reports nothing; a deliberately
broken copy (bad and missing composite inputs, bad and missing reusable inputs, a nonexistent `needs:` target)
is reported in full; `shellcheck` 0.11.0 is clean on all scripts; both test scripts pass, and mutating the
certifier, the gate or the classifier makes them fail.

## How the pieces answer the design questions

**Seams.** A cell needs a runner, timeout, permissions and artifacts -> reusable workflow. The setup steps are
needed by several jobs -> composite action. `deps`, `mutants`, `ci-scripts` are used once -> plain jobs.

**Triggers.** `pull_request`, `merge_group`, `push: main`, `workflow_dispatch`; stale PR runs cancelled, `main`
and queue runs never. No workflow-level `paths:`; `changes` decides.

**Wall-clock.** Each cell is its own job, so cells run in parallel on separate checkouts.
`scripts/contract-all.sh` serialises them on one machine because parallel runs there share
`crates/thrust-worker/build` and a second `wrangler` rebuild reloads the first one's worker (its header);
separate runners remove that coupling. Caches restore on PRs and save from `main` only, keyed per family
(`contract-native`, `contract-wasm`).

**Required check.** Only `gate`. `plan` reads the cell list from the script and fails if it parses zero cells or
if the count of `run ...` lines differs from the count it parsed; each cell uploads `<target>-<pass>.ok` only
after `scripts/contract.sh` exits 0; `gate` compares the planned set with the marker set. A skipped needed job
when Rust changed is a failure (vacuity), and a docs-only PR passes only because `changes` said nothing relevant
changed.

**Hardening.** `permissions: {}` plus per-job `contents: read`; full-SHA pins with version comments;
`persist-credentials: false`; inputs only through `env:`; cache saved on `main` only; pinned tool versions.

## Details worth copying

- `scripts/contract.sh` is a bare command in the workflow step; do **not** wrap it in `| tee` or follow it with
  `echo "exit=$?"`, either of which would replace its status.
- `scripts/flake-hunt.sh` ends with `echo "failures: X / N"`, so **its exit status is always 0**.
  `flake-hunt.yml` runs it into a log and asserts the line `failures: 0 / <N>` exists. Any caller must do the
  same, or a flaky cell reads as green. Its `pass` input is a `choice`, and the validation step rejects
  combinations that cannot run (`cron` on a native target; `worker`/`bindings` off `worker-cf`).
- `native-pg` and `worker-cf` start a Postgres container themselves (`docker run` inside the script) and pick
  free ports at random; they need Docker on the runner. The `contract-cell` preflight step fails with a clear
  message if `curl openssl lsof pgrep docker` is missing, instead of letting the script die obscurely.
- `scripts/rs` puts the rustup toolchain first on `PATH` (and sets a `DYLD_*` fallback that is inert on Linux).
  The composite installs `stable` through rustup, which is what it resolves.
- Worker cells need the `wasm32-unknown-unknown` target, `worker-build` and `wrangler`. The composite takes both
  versions as inputs (defaults `worker-build` 0.8.7 and `wrangler` 4.123.0, the versions the repo's tooling used on
  2026-10-18; `crates/thrust-worker/wrangler.toml` records the `wrangler` it was measured with). Re-check both when
  bumping. `worker-build` is cached by rust-cache with the rest of `~/.cargo`, saved from `main` only.
- The Worker bundle's release profile has a measured constraint (`strip = "debuginfo"`, not `true`, or the bundle
  breaks): it is a property of `Cargo.toml`, not of CI, so CI must build with the repo's profile.

## Not covered, on purpose

- `scripts/contract.sh <target> bench` and `blast` (observational, not part of the matrix).
- The testcontainers adapter tests and the probes (a separate job if wanted; nothing here depends on them).
- Release/deploy workflows, and the mdBook Pages deploy workflow (see `mdbook`).

## Assumptions to confirm before relying on this

1. The `ubuntu-24.04` runner image provides `curl openssl lsof pgrep docker` `[derived]`; the preflight step turns
   a missing tool into a clear failure.
2. `scripts/mutants.sh` kills every mutant on a CI runner. It is plain shell around `cargo test` (it does not use
   `cargo-mutants`) `[unverified: not run]`.
3. First real run: expect to tune `timeout-minutes` and cache keys using the timing query in `wall-clock.md`.
