---
name: dockerfile
description: Use when writing or debugging the `Dockerfile` for the thrust server crate - `docker stop` takes 10 s and exits 137, `GLIBC_2.38 not found`, `cannot stat 'target/release/migrate'`, container `unhealthy` with no curl in a distroless image, every build re-downloads all crates, "pin the FROM images by digest", `docker build --check` warnings (`JSONArgsRecommended`, `SecretsUsedInArgOrEnv`), `cargo-chef` multi-stage setup, `.dockerignore`. NOT for compose files, Kubernetes manifests or Helm charts (see helm-chart), GitHub Actions workflows that build or push images (see gha-workflow-design), Cloudflare Worker bundles (see rust-on-workers), or other Rust crates and non-Rust images.
allowed-tools:
  - Edit(/Dockerfile*)
  - Edit(/.dockerignore)
  - Edit(/docker/**)
author: aRustyDev
version: 0.1.0
---

# Dockerfile for the native Rust server

The `Edit` rule covers all file writes and pre-approves only the root `Dockerfile*`, `.dockerignore` and
`docker/**`. Builds and lint runs are the user's to run: ask, then report their output.

**Preconditions (assumed contract; re-read `main.rs`, its signal handling will change):** `crates/thrust-server`
reads `THRUST_*` and `DATABASE_URL` from the environment, binds `THRUST_LISTEN`, handles only SIGINT and serves
`/healthz`; `crates/thrust-adapters-postgres` has a `migrate` binary with `required-features = ["native"]`; a
root `Cargo.lock` exists. The Worker crates in the workspace are `rust-on-workers`'; the contract suite that
drives the server is `rust-testing`'s.

| Need | Read |
|---|---|
| Stages, `cargo-chef`, `required-features`, cache mounts | `references/multistage-and-caching.md` |
| Runtime base, `USER`, env, signals, `HEALTHCHECK` | `references/runtime-and-health.md` |
| Digests, reproducibility, `.dockerignore`, lint | `references/pinning-and-lint.md` |

## Procedure

1. Stages `chef` -> `planner` (`cargo chef prepare --bin thrust-server`) -> `builder` -> `runtime`
   (`references/examples/Dockerfile`).
2. Builder and runtime share one Debian release; runtime is distroless `cc`; numeric non-root `USER`.
3. Digests: never copy a digest from the example and never invent one. Ask the user to run
   `docker buildx imagetools inspect <ref> --format '{{.Manifest.Digest}}'` and paste the index digest; leave
   a visible placeholder until then.
4. Bake only non-secret defaults (`THRUST_LISTEN=0.0.0.0:8080`); `STOPSIGNAL SIGINT`; exec-form `ENTRYPOINT`.
5. Health: copy `references/examples/healthcheck.rs` to `docker/healthcheck.rs`; no curl in distroless.
6. Keep `.dockerignore` excluding `target`, `.git`, `.env*`, `.dev.vars*`.
7. Ask the user to run `docker build --check -f Dockerfile .` (must exit 0), then a real build.

## Output contract

`Dockerfile`, `.dockerignore`, `docker/healthcheck.rs`, and what was run. Never claim an image builds unless
the user's build output shows it; say "not built". Tags: `[unverified: not run]` = not executed; `[derived]` =
from documentation or source, not executed.

## Example

Q: "`docker stop` takes 10 s and exits 137." A: only SIGINT is handled and PID 1 ignores an unhandled SIGTERM;
set `STOPSIGNAL SIGINT` (`references/runtime-and-health.md`).
