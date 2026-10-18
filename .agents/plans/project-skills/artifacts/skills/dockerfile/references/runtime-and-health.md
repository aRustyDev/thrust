# Runtime image: user, environment, signals, health

Assumed contract: `crates/thrust-server/src/main.rs`, `crates/thrust-api/src/lib.rs` (`/healthz`, `/_info`,
`/internal/*`), `crates/thrust-adapters-postgres/src/bin/migrate.rs`. Re-read them before relying on this
file; the signal handling in particular is expected to change. Tags: `[derived]` = from a file or document,
not executed; `[unverified: not run]` = not executed.

## Runtime base: distroless `cc`, same Debian as the builder

- Distroless images "do not contain package managers, shells or any other programs" (distroless README).
  Variants: `static`, `base`, `base-nossl`, `cc`; tags `latest`, `nonroot`, `debug`, `debug-nonroot`. The
  upstream Rust example builds on `rust:1` and runs on `cc-debian13`.
- Pick `cc`, not `static`: a default `x86_64-unknown-linux-gnu` binary is dynamically linked to glibc and
  `libgcc_s` (unwinding), and the server must keep unwinding (workspace `Cargo.toml` comment). `static` is for
  musl or fully static builds only. `base` lacks libgcc/libstdc++.
- **Debian release must match the builder** (trixie/trixie in the example). A binary linked against a newer
  glibc dies on an older runtime with `GLIBC_2.xx not found`.
- No TLS library is needed today: the server pool (`thrust-adapters-selfhosted`) and `migrate` connect with
  `NoTls`. If TLS is added, re-check which distroless variant ships `libssl` and CA certificates.
- `:debug` variants add a busybox shell for `docker run --entrypoint=sh`; for investigation only, never the
  production tag.
- Distroless images are cosign-signed (keyless); verify before pinning:
  `cosign verify <image> --certificate-oidc-issuer https://accounts.google.com
  --certificate-identity keyless@distroless.iam.gserviceaccount.com` (README) `[unverified: not run]`.

## Non-root

`USER 65532:65532` - numeric. The distroless `nonroot` tag is uid/gid 65532 by convention (confirm with
`docker inspect`) `[unverified: not run]`. Numeric matters because Kubernetes cannot verify that a *named* user
is non-root, so `runAsNonRoot: true` rejects the pod `[derived]`. The server writes nothing to disk (logs go to
stderr), so a read-only root filesystem works; never `chown`/`chmod` in the runtime stage (no shell there).

## Environment and ports (from `main.rs`)

| Variable | Default | Image guidance |
|---|---|---|
| `THRUST_LISTEN` | `127.0.0.1:8080` | **bake `0.0.0.0:8080`**: loopback is unreachable via a published port |
| `THRUST_ADAPTERS` | `memory` | leave to the deployment; `postgres` needs `DATABASE_URL`; unknown values panic |
| `DATABASE_URL` | none (panics if `postgres`) | run-time env or secret only; carries the password |
| `THRUST_ADMIN_TOKEN` | none | run-time secret only |
| `THRUST_TEST_ROUTES` | off | **never set in an image**. When on, `/_info` and `/_test/panic` answer **any caller** (they are gated only by this flag, not by a token), and `/internal/*` additionally needs `x-admin-token` |
| `THRUST_PG_POOL`, `THRUST_CREATE_RATE_LIMIT`, `THRUST_RATE_WINDOW_SECS`, `THRUST_OUTBOX_FASTPATH`, `THRUST_SWEEP_INTERVAL_SECS` | defaults live in `main.rs` | tuning; set per deployment, not in the image |

`ENV`/`ARG` bake values into image history (`docker history`). `docker build --check` reports
`SecretsUsedInArgOrEnv` for names that look like secrets. Pass secrets with `docker run -e NAME` (forwards from
the caller's environment, not argv) or an orchestrator secret.

## Signals: exec-form ENTRYPOINT, and why the example sets STOPSIGNAL SIGINT

- Use **exec form** (`ENTRYPOINT ["/usr/local/bin/thrust-server"]`): the server becomes PID 1 and gets signals
  directly. Shell form wraps it in `sh -c` (and distroless has no shell). Docker's lint rule
  `JSONArgsRecommended` exists for this reason.
- `main.rs` ends with `with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; })`: **SIGINT
  only**. `docker stop` sends SIGTERM by default (Dockerfile reference, STOPSIGNAL). A process running as PID 1
  of a container does not get the kernel's default action for a signal it has no handler for, so SIGTERM is
  **ignored**, `docker stop` waits out its timeout (10 s default) and SIGKILLs (exit 137), and in-flight
  requests are cut instead of drained `[unverified: not run; standard PID 1 semantics plus the handler list
  read from main.rs]`.
- Fixes, pick one:
  1. `STOPSIGNAL SIGINT` in the Dockerfile (the example). No code change; `docker stop` then sends SIGINT.
     (`STOPSIGNAL` does not affect Ctrl+C from a terminal.)
  2. Handle SIGTERM in `main.rs` as well (proposal): `tokio` already enables the `signal` feature in
     `thrust-server`'s manifest, so `tokio::signal::unix::signal(SignalKind::terminate())` is available;
     `select!` it with `ctrl_c()`. Once the server handles SIGTERM, `STOPSIGNAL` can be dropped.
  3. Run under an init (`docker run --init`, compose `init: true`): the init is PID 1 and forwards SIGTERM to
     the server, which is then an ordinary process and dies on the default action (no graceful drain, because
     the server never handles SIGTERM).
- Kubernetes: the kubelet asks the container runtime to stop the container; the runtime sends the stop signal
  (the image's `STOPSIGNAL` when set; containerd and CRI-O default to SIGTERM) and kills after
  `terminationGracePeriodSeconds` (30 s default) `[derived]`: the same hang, just longer.

## HEALTHCHECK

Syntax and defaults (Dockerfile reference):

```dockerfile
HEALTHCHECK [--interval=30s] [--timeout=30s] [--start-period=0s] [--start-interval=5s] [--retries=3] \
  CMD ["exec", "form"]
```

- Exit codes: `0` healthy, `1` unhealthy, `2` reserved. Only the **last** `HEALTHCHECK` in a Dockerfile counts;
  `HEALTHCHECK NONE` disables an inherited one.
- `--start-interval` needs Docker Engine 25.0+. During `--start-period` failures do not count toward retries,
  but the first success ends the start period.
- A probe that exceeds `--timeout` is SIGKILLed and counts as a failure. Stdout/stderr (first 4096 bytes) is
  stored and shown by `docker inspect`.
- **Kubernetes ignores `HEALTHCHECK`.** Orchestrated deployments use `httpGet` probes against `/healthz`; the
  Dockerfile `HEALTHCHECK` serves `docker run` and compose.
- `/healthz` returns `"ok"` unconditionally (`thrust-api` `healthz()`): it shows the HTTP server answers, **not**
  that Postgres is reachable. Do not describe it as readiness.

### Probing without curl (the distroless case)

`curl` and `wget` are absent, so `CMD curl -f ... || exit 1` can never pass. The server has no `healthcheck`
subcommand today. Ship a tiny probe: `references/examples/healthcheck.rs` (std only, one file, no cargo),
copied to `docker/healthcheck.rs`. Compiled with rustc 1.98.1 and run against throwaway local servers:

| Case | Exit |
|---|---|
| HTTP 200 | 0 |
| HTTP 404 | 1 |
| connection refused | 1 |
| accepts the connection, never answers | 1, after 2 s (its own timeout) |
| missing arguments | 1 |

Keep the probe's internal 2 s timeout below `--timeout=3s`. The example Dockerfile builds it in a `probe` stage
with `rustc`, so no extra crate or lockfile entry is needed. A future `thrust-server healthcheck` subcommand
would remove the extra binary.
