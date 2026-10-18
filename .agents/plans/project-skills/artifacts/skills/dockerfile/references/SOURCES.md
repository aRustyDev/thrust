# Sources: dockerfile

External sources are cited for facts and short syntax snippets only; no third-party skill text is reproduced.
External sources retrieved 2026-10-18. Repository files are cited by path.

## Repository files (assumed contract)

| Path | Supports |
|---|---|
| `crates/thrust-server/src/main.rs` | environment variables and defaults, `127.0.0.1:8080` default, SIGINT-only shutdown, `THRUST_TEST_ROUTES` |
| `crates/thrust-server/Cargo.toml` | tokio `signal` feature; dependency on the selfhosted and postgres adapters |
| `crates/thrust-adapters-postgres/Cargo.toml`, `src/bin/migrate.rs`, `src/lib.rs` | `required-features = ["native"]`, `DATABASE_URL` from the environment only, embedded SQL, `NoTls` |
| `crates/thrust-adapters-selfhosted/Cargo.toml`, `src/pg.rs` | postgres adapter enabled with `native`; pool uses `NoTls` |
| `crates/thrust-api/src/lib.rs` | `/healthz` returns `"ok"`; `/_info` and `/_test/panic` gated by the test-routes flag; `/internal/*` also by `x-admin-token` |
| workspace `Cargo.toml` | wasm-tuned release profile; unwinding required by `CatchPanicLayer` |

## External sources

| Source | Supports | Licence |
|---|---|---|
| https://github.com/LukeMathWalker/cargo-chef (README, `src/main.rs`) | stage layout, `prepare --bin`, `cook` flags, same-workdir and path-dependency caveats | Apache-2.0 OR MIT |
| https://docs.docker.com/build/checks/ | `--check` semantics, exit status, `check=` directives, buildx >= 0.15 | Apache-2.0 (docker/docs) |
| https://docs.docker.com/build/building/best-practices/ | digest pinning, Dependabot `docker` ecosystem, `.dockerignore` | Apache-2.0 (docker/docs) |
| https://github.com/moby/buildkit/blob/master/frontend/dockerfile/docs/reference.md | `HEALTHCHECK` options, defaults and exit codes; `STOPSIGNAL`; cache-mount options and the local-storage note | Apache-2.0 |
| https://docs.docker.com/build/ci/github-actions/reproducible-builds/ | `SOURCE_DATE_EPOCH` | Apache-2.0 (docker/docs) |
| https://github.com/GoogleContainerTools/distroless (README, `examples/rust/Dockerfile`) | image variants, no shell, `:debug`, cosign verification, Rust example uses `cc` | Apache-2.0 |
| https://github.com/hadolint/hadolint (README) | flags, config search path, rule ids (no code reused) | GPL-3.0 |

## Verification status

| Claim group | Status |
|---|---|
| Base-image digests and the `# syntax` frontend digest in `examples/Dockerfile` | resolved 2026-10-18 with `docker buildx imagetools inspect <ref> --format '{{.Manifest.Digest}}'`; a snapshot |
| `-p pg` skips a `required-features` bin; `--bin migrate` errors; `-p server -p pg` builds both (feature unification); `-p server -p pg --features pg/native` builds both | executed with cargo 1.98.1 on a minimal workspace |
| `healthcheck.rs` exits 0 / 1 / 1 / 1 / 1 for 200, 404, refused, silent server, no arguments | compiled with rustc 1.98.1 and run against throwaway local servers |
| `docker build --check` on `examples/Dockerfile`; an image build; `cargo chef` commands; PID 1 ignoring an unhandled SIGTERM; hadolint; cosign | `[unverified: not run]` |
| Kubernetes behaviours (`runAsNonRoot` with a named user, runtime stop signal) | `[derived]` |

## Not reproduced

Floating image tags without digests, secrets in `ENV`/`ARG`, `curl | sh` installers in build stages, `:debug`
images as production tags.
