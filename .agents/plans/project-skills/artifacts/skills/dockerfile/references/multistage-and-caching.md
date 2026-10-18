# Multi-stage layout, cargo-chef, and cache mounts

Tags: `[derived]` = from a document or source file named in `SOURCES.md`, not executed;
`[unverified: not run]` = not executed. Unmarked behaviour was run on a minimal workspace (see `SOURCES.md`).

## Stage layout (`examples/Dockerfile`)

```text
chef      FROM cargo-chef image (rust toolchain + cargo-chef), WORKDIR /app
planner   COPY . .  ->  cargo chef prepare --recipe-path recipe.json --bin thrust-server
builder   COPY recipe.json  ->  cargo chef cook ...   (cached layer: dependencies only)
          COPY . .          ->  cargo build ...       (our crates; reruns on any source change)
probe     rustc a std-only health probe (needed by the HEALTHCHECK)
runtime   FROM distroless cc, COPY the binaries, USER 65532, HEALTHCHECK, ENTRYPOINT
```

Why the split works: `recipe.json` captures the manifests and lockfile only. While dependencies are unchanged
the recipe is byte-identical, so the `cook` layer is a cache hit even though `COPY . .` in `planner` was
invalidated by a source edit.

## cargo-chef facts that matter for this workspace

- **`prepare --bin <name>` prunes the recipe to what that binary needs** `[derived: cargo-chef source, CLI help
  "ignore all members of the workspace that are not necessary to successfully compile the specific
  binary"]`. The workspace also contains the wasm32 Worker crate; without `--bin` it is in the recipe and
  `cook` tries to build it for the host.
- `cook` accepts cargo's selection flags: `--package/-p`, `--bin`, `--bins`, `--workspace`, `--features`,
  `--no-default-features`, `--profile`, `--target`, `--locked`, `--offline` `[derived: same source file]`. Pass
  **exactly the selection the final `cargo build` uses**, or the cooked layer is for a different feature set
  and the dependencies rebuild.
- `cook` and `build` must run from the **same working directory** (`/app`): dependency `*.d` files embed
  absolute paths `[derived: cargo-chef README caveats]`.
- Local path dependencies are always rebuilt (cargo fingerprints by mtime), so workspace crates are never
  cached; only registry and git dependencies are `[derived: README]`.
- `Cargo.lock` must be committed and `--locked` belongs on every cargo line.
- `cargo chef` commands themselves: `[unverified: not run]`.

## The migrate binary and `required-features`

`crates/thrust-adapters-postgres/Cargo.toml` declares `[[bin]] name = "migrate"` with
`required-features = ["native"]`. The server depends on `thrust-adapters-selfhosted`, which depends on the
postgres adapter with `features = ["native"]`. Cargo unifies features across the selected packages, so the
feature is already on whenever `thrust-server` is part of the build. Behaviour on a minimal workspace of the
same shape (`server` -> `pg[native]`, `pg` has a `required-features` bin):

| Command | Result |
|---|---|
| `cargo build --release -p pg` | exit 0, **no `migrate` binary** (the feature is off: `pg` alone selected) |
| `cargo build --release -p pg --bin migrate` | error: `target migrate in package pg requires the features: native` |
| `cargo build --release -p server -p pg` | **both binaries built**, no `--features` needed (unification through `server`) |
| `cargo build --release -p server -p pg --features pg/native` | both binaries built |

So the Dockerfile's `--features thrust-adapters-postgres/native` is **defensive**: it keeps `migrate` building
if the server stops depending on the adapter's `native` feature. The scenario "cargo said Finished but
`target/release/migrate` is missing" reproduces only when the adapter is built **without** `thrust-server` in
the same invocation. The `install -D` after the build fails loudly if `migrate` is missing. Building both in one
invocation also lets shared dependencies compile once with one feature set `[unverified: not timed]`.

The migration SQL is embedded (`include_str!("../migrations/0001_init.sql")` in the adapter's `lib.rs`), so
the runtime image needs no `.sql` files. `migrate` reads `DATABASE_URL` from the environment only; never bake it.

## The release profile is tuned for wasm, not for this image

The workspace `[profile.release]` (read it in the root `Cargo.toml`) is commented as the wasm bundle's
size-measurement profile: size-optimised, fat LTO, a single codegen unit. In a container build that means slow,
size-optimised native binaries. Options, both `[unverified: not run]`:

- Add a profile (`[profile.docker] inherits = "release"`, `lto = "thin"`, `codegen-units = 16`,
  `opt-level = 3`) and pass `--profile docker` to **both** `cook` and `build` (cargo-chef supports `--profile`);
  binaries then live in `target/docker/`, so change the `install` paths.
- Or env overrides (`CARGO_PROFILE_RELEASE_LTO=thin`) on both lines. A mismatch between the two lines silently
  defeats the cache.

Do not set `panic = "abort"` for the native server: the manifest comment says unwinding is required so
`CatchPanicLayer` can answer 500s. That is also why the runtime is distroless `cc` (libgcc_s) rather than
`static`.

## BuildKit cache mounts

Syntax `[derived: Dockerfile reference]`: `RUN --mount=type=cache,target=<dir>[,id=][,sharing=shared|private|locked]
[,uid=][,gid=][,mode=]`; `id` defaults to the target; default `sharing` is `shared`; `uid`/`gid` default 0.
"Cache mounts should only be used for better performance": the build must work with an empty directory.

For cargo, mount `/usr/local/cargo/registry` and `/usr/local/cargo/git` (CARGO_HOME of the official `rust`
images, which the cargo-chef images build on). The example uses `sharing=locked`, which makes a second
concurrent writer wait (Docker's own examples use it for apt, which needs exclusive access); cargo takes its own
lock on the package cache, so `shared` is probably also safe `[unverified: not run]`.

**Limitation `[derived: Dockerfile reference note]`:** cache mounts live in the *builder's local storage*. They
are not part of exported build cache (`--cache-to`) and are not restored by `--cache-from`. On CI runners that
start with a fresh builder they are empty on every build.

| Goal | Use |
|---|---|
| Fast rebuilds on a developer machine or long-lived builder | cargo-chef layers **plus** registry cache mounts (the example) |
| Fast rebuilds on ephemeral CI | cargo-chef layers exported with `--cache-to` / `--cache-from` (for example `type=gha,mode=max`; `mode=max` exports intermediate stages) `[unverified: not run]` |
| No cargo-chef | `RUN --mount=type=cache,target=/app/target cargo build --release && cp target/release/<bin> /out/` (copy artifacts out inside the same `RUN`; the mount is not in the image); local builders only |

## Things that look like speedups and are not

- `COPY . .` before `cook`: invalidates the dependency layer on every edit.
- Skipping `.dockerignore`: a host `target/` in the context is sent every build and busts `COPY . .`.
- `cargo build` in `planner`: the planner must stay cheap; it only emits `recipe.json`.
- A floating `latest` chef tag without a digest: the toolchain changes under you and the cache key with it
  (`pinning-and-lint.md`).
