# Actions

One row per composite action in this directory. How each is used, and why, is in
[CI/CD](../../docs/src/architecture/cicd.md); this is the index, and `.github/scripts/check-ci-index.sh` fails (in the `deps`
job) when an action has no row here or a row names a directory that is not here.

An action is a set of steps that two or more jobs would otherwise repeat. A steps-only action has no runner, timeout or
permissions of its own: those belong to the job that uses it, so a job that needs them is a workflow
(`../workflows/contract-cell.yml`), not an action.

| Action | Used by | Does |
|---|---|---|
| `build-bundle` | `deploy.yml`, `release.yml` | The deploy bundle of the checked-out commit (the Worker, the `migrate` binary and the web app) in one reproducible file, so that a deploy and a release build it the same way |
| `build-native` | `build.yml` (its job `linux`), `build-extended.yml` (its job `macos`) | A compile proof of the native server and `migrate`, in release, for the target of the runner it runs on: it checks that the compiler's host is the target the job claims, builds both with the repository's commands (`migrate` with the one `build-bundle` uses), checks that both binaries exist, and uploads them for three days. A job gives it a name and a target; the runner is the job's |
| `contract-pass-tools` | `contract-cell.yml`, `flake-hunt.yml` | What one pass of `scripts/contract.sh` needs beyond Rust (`uv`, Node, pnpm, the built app, Chromium), then a check that every host tool it relies on exists |
| `deploy-worker` | `deploy.yml` | Fetch the bundle, verify its attestation, unpack it, install wrangler and `cloudflared` at pinned versions and run `scripts/deploy-worker.sh` |
| `setup-rust` | `build.yml` (its `wasm` job, and `build-native`, which `build-extended.yml` uses too), `ci.yml`, `contract-cell.yml`, `flake-hunt.yml`, `lint.yml`, `release-plz.yml`, `security.yml`, `test.yml` | The Rust version `rust-toolchain.toml` pins, its targets and components, a keyed cache, and optionally the Worker toolchain and `cargo-nextest` |
| `triggerd` | nothing yet | Bounded policy-based selection of the checks a change needs (a TypeScript action, `dist/` checked in). Present and not enabled: the planner in use is `.github/scripts/plan-ci.py` |
| `verdict` | `documentation.yml`, `security.yml`, `web.yml`, `lint.yml`, `test.yml`, `build.yml` | The judge at the end of a called workflow (ADR-0102): runs `assert-gate.sh` over the workflow's own jobs against the plan it was given, and only then sets the output `verdict=pass` that `ci.yml`'s `gate` requires. A workflow that runs the contract cells (`test.yml`) also gives the cell job and the planned cells: the action then collects the cells' pass markers and the judge proves that each planned cell reported. The calling job checks the repository out first: a local action is read from the checkout |
