# syntax=docker/dockerfile:1@sha256:4edf897a3ffa55b89f906fc8cc78afdb3f1834cc9c7083565e611a8a7d5fe99e
# check=error=true

# DIGESTS BELOW WERE RESOLVED 2026-10-18 AND ARE ILLUSTRATIONS. Refresh them before use and never copy them
# into another project: resolve your own with `docker buildx imagetools inspect <ref> --format
# '{{.Manifest.Digest}}'` (the multi-arch index digest). The `# syntax` line pins the Dockerfile frontend the
# same way.
#
# Every FROM is written literally as tag@digest so update bots (Dependabot, package-ecosystem: docker) can see
# and bump it. Builder and runtime MUST be the same Debian release (trixie here): a binary linked on a newer
# glibc fails on an older runtime with `GLIBC_2.xx not found`.

FROM lukemathwalker/cargo-chef:latest-rust-1.99-slim-trixie@sha256:b22df2bf8abec048f46a6505c9303e2c8c0a8e22d21181f8b803b9ad5593ba99 AS chef
WORKDIR /app

# planner: recipe.json holds only what dependency resolution needs. `--bin` drops workspace members the
# server does not depend on (the wasm32 Worker crate cannot be cooked natively).
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json --bin thrust-server

# builder: dependencies first (layer cached while recipe.json is unchanged), then our code.
# Server and migrate are built in ONE invocation so shared dependencies compile once. `migrate` has
# required-features = ["native"]; the server's dependency graph already enables it, so `--features` is
# defensive: `-p thrust-adapters-postgres` ALONE would silently skip `migrate`.
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo chef cook --release --locked --recipe-path recipe.json \
      -p thrust-server -p thrust-adapters-postgres --features thrust-adapters-postgres/native
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --release --locked \
      -p thrust-server -p thrust-adapters-postgres --features thrust-adapters-postgres/native \
 && install -D -m 0755 target/release/thrust-server /out/thrust-server \
 && install -D -m 0755 target/release/migrate /out/migrate

# probe: std-only HTTP probe for an image with no shell and no curl. The HEALTHCHECK below needs it; to drop
# the probe, delete this stage, the `COPY --from=probe` line and the HEALTHCHECK together.
# Copy references/examples/healthcheck.rs to docker/healthcheck.rs first.
FROM chef AS probe
COPY docker/healthcheck.rs /src/healthcheck.rs
RUN mkdir -p /out && rustc --edition 2021 -C opt-level=s -C strip=symbols /src/healthcheck.rs -o /out/healthcheck

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:e792ab3d241a468a4fd7519ddbbebe66b49b5f365771716ea688ad40b6c6f1c2 AS runtime
COPY --from=builder /out/thrust-server /usr/local/bin/thrust-server
COPY --from=builder /out/migrate /usr/local/bin/migrate
COPY --from=probe /out/healthcheck /usr/local/bin/healthcheck
# The server defaults to 127.0.0.1:8080, which is unreachable through a published port.
# No DATABASE_URL / THRUST_ADMIN_TOKEN here: secrets arrive at run time, never in an ENV or ARG.
ENV THRUST_LISTEN=0.0.0.0:8080
EXPOSE 8080
USER 65532:65532
# main.rs installs only a SIGINT handler (ctrl_c); as PID 1 an unhandled SIGTERM is ignored.
STOPSIGNAL SIGINT
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --start-interval=2s --retries=3 \
  CMD ["/usr/local/bin/healthcheck", "127.0.0.1:8080", "/healthz"]
ENTRYPOINT ["/usr/local/bin/thrust-server"]
