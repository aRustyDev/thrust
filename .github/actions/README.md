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
| `contract-pass-tools` | `contract-cell.yml`, `flake-hunt.yml` | What one pass of `scripts/contract.sh` needs beyond Rust (`uv`, Node, pnpm, the built app, Chromium), then a check that every host tool it relies on exists |
| `deploy-worker` | `deploy.yml` | Fetch the bundle, verify its attestation, unpack it, install wrangler and `cloudflared` at pinned versions and run `scripts/deploy-worker.sh` |
| `setup-rust` | `ci.yml`, `contract-cell.yml`, `flake-hunt.yml`, `release-plz.yml`, `security.yml` | The Rust version `rust-toolchain.toml` pins, its targets and components, a keyed cache, and optionally the Worker toolchain and `cargo-nextest` |
| `triggerd` | nothing yet | Bounded policy-based selection of the checks a change needs (a TypeScript action, `dist/` checked in). Present and not enabled: the planner in use is `.github/scripts/plan-ci.py` |
| `verdict` | `documentation.yml`, `security.yml` | The judge at the end of a called workflow (ADR-0102): runs `assert-gate.sh` over the workflow's own jobs against the plan it was given, and only then sets the output `verdict=pass` that `ci.yml`'s `gate` requires. The calling job checks the repository out first: a local action is read from the checkout |
