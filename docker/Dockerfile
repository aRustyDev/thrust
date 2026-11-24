# check=error=true

# The self-hosted server and its migration binary, as one small image. Build it from the repository root:
#
#   docker build -f docker/Dockerfile -t thrust-server .
#   docker run --rm -p 8080:8080 thrust-server          # the memory adapters; /healthz answers
#   docker run --rm --entrypoint /usr/local/bin/migrate -e DATABASE_URL thrust-server
#
# The `docker` job of .github/workflows/build-extended.yml builds it every night and for a release candidate, starts it once and publishes
# nothing (Build phase, stage 5: .agents/plans/ci-workflows/phases/0004-build.md has the measurements and the owner's choices).
#
# * ONE PIN FOR THE COMPILER: rust-toolchain.toml. The builder is named for its operating system (`alpine3.24`), not for a Rust version, and
#   rustup installs the compiler the file pins on top of the image's own when they differ (measured: pinning 1.99.1 downloaded it in 13 s,
#   and a file that matches the image changes nothing). So Dependabot's weekly bump of the file changes this image's compiler too, and
#   nothing here has to follow it. `--profile minimal` is what a build needs (no docs, no clippy, no rustfmt).
# * STATIC, ON MUSL. The builder's own target is x86_64-unknown-linux-musl, which links statically, so the runtime image has no libc
#   (`distroless/static`): the binaries are all that is in it, besides the CA bundle and the `nonroot` user. The server's TLS is rustls, with no
#   OpenSSL: the web roots of OIDC and SMTP are compiled in (webpki-roots, through reqwest and lettre), and Postgres' `verify-ca` reads the
#   operator's own CA file at run time, so that file is mounted into the container, not baked into it. The owner chose this over glibc
#   (`distroless/cc`) on 2026-11-23. On a hosted runner, cold, the job took 324 s and 236 s on musl (two runs) and 149 s on glibc (one), the two cargo builds
#   307 s and 213 s against 115 s: musl's compile is about twice as long (1.8 to 2.65 times), which an emulated run on a Mac had put at 1.3. The image is
#   15.4 MB against 40.5 MB (`docker image ls`).
#   THE RISK THAT CHOICE TAKES, not measured for this server: musl's default allocator is reported
#   to be slower than glibc's under multi-threaded load (public posts say 25% to several times), and its DNS resolver differs. The remedy for
#   the allocator is a global allocator in `thrust-server` (a code change, not this file's); if a load test says so, glibc is a two-line change.
# * THE BASES ARE PINNED BY DIGEST (the index, so it covers every platform), resolved 2026-11-23 with
#     docker buildx imagetools inspect <ref> --format '{{.Manifest.Digest}}'
#   Nothing bumps them: .github/dependabot.yml has no `docker` ecosystem (adding one is its own decision). Re-resolve them when the tag moves.
# * RUNS AS 65532:65532, numeric (the `nonroot` user of distroless), so that Kubernetes' `runAsNonRoot` can verify it. The server writes nothing to
#   disk, so a read-only root filesystem works.
# * `STOPSIGNAL SIGINT`: the server handles SIGINT only (`tokio::signal::ctrl_c` in crates/server/src/main.rs), and as PID 1 it ignores SIGTERM
#   (measured: `docker kill -s TERM` left it running; `docker stop` with SIGINT stopped it in under a second, exit 0). Handling SIGTERM is a code
#   change in the server; when it lands, this line can go.
# * NO HEALTHCHECK, on purpose. The image has no shell and no curl, and Kubernetes ignores the instruction (it probes /healthz itself).
#   `/healthz` answers "ok" whenever the HTTP server does: it is not a readiness check of the database.
# * NO SECRET AND NO SETTING of the deployment is baked in. DATABASE_URL, THRUST_ADAPTERS and the tokens arrive at run time. THRUST_TEST_ROUTES
#   must never be set in an image: it opens /_info and /_test/panic to any caller.

FROM rust:alpine3.24@sha256:8d0e82abfdf9a38071e07cabc5afba4ddbe077631a80e4e742a760c38beadc33 AS build
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup toolchain install --profile minimal --no-self-update
COPY . .
# The two commands of the `build-native` action (.github/actions/build-native), which prove the same build on the runner's own target; the
# workflow wiring test holds them equal. `migrate` needs the `native` feature, which the server's own build does not turn on for it.
RUN cargo build --release --locked -p thrust-server \
 && cargo build --release --locked -p thrust-adapters-postgres --features native --bin migrate \
 && mkdir /out \
 && cp target/release/thrust-server target/release/migrate /out/

FROM gcr.io/distroless/static-debian13:nonroot@sha256:8ce6dd87e9823c5b48dcfb2c8604714ccb351ce59607327bcc4957448da279b5
COPY --from=build /out/thrust-server /out/migrate /usr/local/bin/
# The server binds 127.0.0.1:8080 unless told otherwise, which a published port cannot reach.
ENV THRUST_LISTEN=0.0.0.0:8080
EXPOSE 8080
USER 65532:65532
STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/local/bin/thrust-server"]
