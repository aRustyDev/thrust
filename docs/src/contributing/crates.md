# Crates

Where a change belongs in the Cargo workspace, and what adding a crate involves. What each crate is and how they
depend on one another is on [Application architecture](../architecture/app.md); this page does not repeat it.

## Which crate does a change belong in?

| You are adding | It goes in | Constraint |
|---|---|---|
| An identifier, domain record or event shared across layers | `thrust-contracts` | Depends on serde, serde_json, uuid and chrono only. chrono has no `clock` feature there |
| A capability the core needs from outside: a store, a queue, time | a trait in `thrust-ports` | Every returned future is `Send`. [Patterns](patterns.md) has the steps |
| A business rule, a use-case or a permission check | `thrust-core` | Reaches the outside only through the `Ports` bundle |
| An HTTP route, or a request, response or error DTO owned by this surface | the owning HTTP crate, such as `thrust-api` | The product handlers call a core use-case and never see a platform type. Only the test-route handlers reach `app.ports` directly. Do not move an HTTP DTO inward merely because it crosses the wire |
| The text of a reproduced Plane 500, or the function that answers it | `thrust-plane-types` | Both Plane-shaped crates (`thrust-api-v1`, `thrust-compat-plane`) answer a 500 through `plane_500` and write none of their own: `scripts/check-plane-500.sh` rejects `StatusCode::INTERNAL_SERVER_ERROR` and its spellings in either, and CI's `deps` job runs it. Depends on `thrust-api` and axum only |
| SQL | `thrust-adapters-postgres` | Written once, against `tokio_postgres::Client`, behind a `Connector` |
| Code that only runs natively | `thrust-adapters-selfhosted` | Never compiled into the Worker |
| Code that uses a Workers binding (R2, Queues, Hyperdrive, a Durable Object client) | `thrust-adapters-cloudflare` | wasm32 only. The Durable Object classes themselves live in `thrust-worker`, because the Wasm module that exports them must also export `fetch` |
| A reference implementation of a port | `thrust-adapters-memory` | Compiles for wasm32 and native with no runtime dependency, because both roots can include it |
| Reading configuration, choosing adapters, starting the process | a root: `thrust-server` or `thrust-worker` | Where the application reads its configuration from the environment. Core, the router and the ports read none; the `migrate` binary and the contract suite read their own variables |
| An HTTP assertion that must hold on every target | `thrust-contract-tests` | Depends on `thrust-contracts` only, so it never links the code it tests |

## The portable crates are a list you maintain

`thrust-contracts`, `thrust-ports`, `thrust-core`, `thrust-api`, `thrust-plane-types`, `thrust-api-v1` and
`thrust-compat-plane` must not depend, even indirectly, on a runtime or platform crate. `scripts/check-deps.sh` checks that for exactly the crates named in its `CLEAN_CRATES` variable, and
checks `thrust-adapters-memory` separately. A new crate that has to stay portable is not covered until you add it to
that list, and CI's `deps` job runs the same script, so it does not cover that crate either. The check is also a second list: it looks only for the 17 crate names in its `FORBIDDEN` variable, in
normal dependencies, so a platform crate that is not on it passes. The ORM crates are the exception: the script asks
the whole workspace for them, on every kind of edge. How the check works and how to prove it can fail
are on [Architecture gates](../testing/techniques/architecture-gates.md).

HTTP surfaces point inward: a surface-owned DTO stays in its owning HTTP crate, and `thrust-contracts`,
`thrust-ports` and `thrust-core` must never depend on `thrust-compat-plane`. `scripts/check-deps.sh` confirms that
`thrust-compat-plane` is a workspace member and fails closed unless `cargo tree` explicitly says it is absent from
each of those three graphs.

## Adding a crate

1. Choose its directory (ADR-0009). A library goes in the group for its layer:
   - `core/` for data types, port traits and use-cases; nothing in it depends on a crate outside it.
   - `api/` for an HTTP surface.
   - `adapters/` for port implementations.
   - `editor/` for the editor.

   Other crates have their own places:
   - A program sits at the top of `crates/`. `crates/README.md` lists the names reserved for programs.
   - A spike that must build against the workspace's lockfile goes in `spikes/`.
   - A crate that only tests goes in `testing/suites/` or `testing/fixtures/`. No crate depends on it except as a
     dev-dependency.

   The directory drops the `thrust-` prefix, and the package keeps it: `crates/<group>/<dir>/` holds `thrust-<name>`.
   Name the crate in its group's `README.md` as `` `<dir>/` ``. A directory is a crate or a group, never both, so a new
   group gets a `README.md` of its own. `scripts/check-layout.sh`, in CI's `deps` job, refuses a crate that its group's
   `README.md` does not name, a directory under `crates/` that is neither, and a dependency the layout forbids.
2. Create `crates/<group>/<dir>/Cargo.toml`. All 21 existing crates take `edition`, `version`, `license` and `publish`
   from the workspace (`edition.workspace = true` and so on), so the licence (`AGPL-3.0-only`) and `publish = false`
   stay in one place. Add the directory to `members` in the root `Cargo.toml`. The list is explicit, with no glob.
3. If another crate will depend on it, add `thrust-<name> = { path = "crates/<group>/<dir>" }` under
   `[workspace.dependencies]` and depend on it with `{ workspace = true }`, never with a `path` of its own: then the
   root `Cargo.toml` is the only file that names a crate's directory. Fifteen crates are declared that way: contracts,
   ports, core, api, plane-types, api-v1, compat-plane, the five adapter crates, `thrust-editor-doc`,
   `thrust-oracle-fixture` and `thrust-contract-tests` (a dev-dependency of `thrust-load-tests`). Nothing depends on the
   three programs (`thrust-server`, `thrust-worker`, `thrust-edge`), `thrust-load-tests`, `thrust-profiling` or `thrust-editor-worker`, so
   they have no entry. A workspace entry that turns default
   features off (`thrust-editor-doc`'s does) is the only place that can: Cargo ignores a member's own
   `default-features = false` on an inherited dependency.
4. Put the crate's tests in the crate. cargo-mutants runs only the mutated package's own tests, and
   `cargo nextest run -p <crate>` selects nothing, and exits 4, for a crate with no tests of its own. The reasoning is
   Rule 5 in [Rules](../testing/rules.md). A test that reads a file outside its crate (the oracle's corpus, the deploy's
   CA root, `target/`) builds the path from `env!("THRUST_ROOT")`, the workspace root that `.cargo/config.toml` sets,
   never from `CARGO_MANIFEST_DIR` and `../..`, so it reads the same file wherever the crate sits (ADR-0009).
5. If the crate has Cargo features, read [Cargo features](features.md) first.
6. A crate that builds for wasm32 as a Worker is a `cdylib`, as `thrust-worker`, `thrust-edge` and `thrust-editor-worker` are, and
   is built by `worker-build` through `scripts/rs`, which puts rustup's toolchain (with the wasm32 standard
   library) ahead of Homebrew's `rustc`. The comment in `crates/worker/wrangler.toml` says so.
7. If the crate is `thrust-core`, `thrust-ports`, an adapter or one of the Plane-shaped crates (`thrust-api-v1`,
   `thrust-plane-types`, `thrust-compat-plane`), give it `[lints]` with `workspace = true`. That is what
   makes clippy deny `unwrap`, `expect`, `panic!` and indexing in it (ADR-0006's panic policy). A crate that does not
   opt in is linted for warnings only, and nothing reports that it was left out.

## Not part of the product graph

- **The editor spike's Worker.** `thrust-editor-worker` (`crates/spikes/editor-worker`) is Spike 0006's measurement
  Worker: a bare Worker that measures the size and CPU of `thrust-editor-doc`. It is a workspace member so that it
  builds against the workspace's lockfile (ADR-0009 rule 5). It depends only on `thrust-editor-doc`, no crate depends
  on it, and no workflow or script builds or deploys it. `thrust-editor-doc` (`crates/editor/doc`) began in the same
  spike and is product code now: `thrust-api-v1` and `thrust-compat-plane` use its sanitiser (the `sanitize` feature).
  It depends on no thrust crate and keeps its own tests. The default nextest profile runs them, except its four
  `oracle_` tests, which are opt-in.
- **`crates/testing/`.** `thrust-contract-tests` (the HTTP contract suite) and `thrust-oracle-fixture` (the v1
  oracle's seed) never ship. The suite tests a running target from outside, and the fixture is only ever a
  dev-dependency (ADR-0009 rule 6).
- **`probes/`.** `send-shape` (which port-trait shape lets one axum router compile on native and wasm32), `sqlx-wasm`
  (whether sqlx builds for wasm32; `scripts/verify-all.sh` treats a compile failure there as the expected result)
  and `ammonia-400` (the editor sanitiser's configuration against ammonia 4.0.0). Each has its own `[workspace]`, so
  none is a member of the root workspace, and each has a `run.sh`.
- **`tools/spike-0006/`.** The TypeScript and Python oracle, corpus generators and benchmarks for the editor spike.
  How it is used is on [Differential](../testing/techniques/differential.md).
- **`tools/compat-inventory/`.** The static inventory of Plane's internal API for spike 0009: stdlib-only Python,
  run through `uv`. Its pinned output is the F-X13 acceptance list. Its README gives the source of the copy and the
  command that reproduces that output.
- **`tools/spike-0004/`.** The harness for spike 0004: three throwaway JavaScript Workers, an OpenTofu root and a
  Python assertion suite. Its `run_platform.sh` creates real Cloudflare resources, so read its README first.
