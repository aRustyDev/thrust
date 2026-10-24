# Crates

Where a change belongs in the Cargo workspace, and what adding a crate involves. What each crate is and how they
depend on one another is on [Application architecture](../architecture/app.md); this page does not repeat it.

## Which crate does a change belong in?

| You are adding | It goes in | Constraint |
|---|---|---|
| A type that crosses a boundary: an id, a record, an event, a request or error body | `thrust-contracts` | Depends on serde, serde_json, uuid and chrono only. chrono has no `clock` feature there |
| A capability the core needs from outside: a store, a queue, time | a trait in `thrust-ports` | Every returned future is `Send`. [Patterns](patterns.md) has the steps |
| A business rule, a use-case or a permission check | `thrust-core` | Reaches the outside only through the `Ports` bundle |
| An HTTP route or its error mapping | `thrust-api` | The product handlers call a core use-case and never see a platform type. Only the test-route handlers reach `app.ports` directly |
| SQL | `thrust-adapters-postgres` | Written once, against `tokio_postgres::Client`, behind a `Connector` |
| SQLite SQL (spike 0002) | `thrust-adapters-sqlite` | Written once, against the synchronous `Sql` trait: rusqlite natively (feature `native`, never for wasm32) and `DoSql` (in `thrust-adapters-cloudflare`) inside the Durable Object. Opt-in until ADR-0040 decides |
| Code that only runs natively | `thrust-adapters-selfhosted` | Never compiled into the Worker |
| Code that uses a Workers binding (R2, Queues, Hyperdrive, a Durable Object client) | `thrust-adapters-cloudflare` | wasm32 only. The Durable Object classes themselves live in `thrust-worker`, because the Wasm module that exports them must also export `fetch` |
| A reference implementation of a port | `thrust-adapters-memory` | Compiles for wasm32 and native with no runtime dependency, because both roots can include it |
| Reading configuration, choosing adapters, starting the process | a root: `thrust-server` or `thrust-worker` | Where the application reads its configuration from the environment. Core, the router and the ports read none; the `migrate` binary and the contract suite read their own variables |
| An HTTP assertion that must hold on every target | `thrust-contract-tests` | Depends on `thrust-contracts` only, so it never links the code it tests |

## The portable crates are a list you maintain

`thrust-contracts`, `thrust-ports`, `thrust-core` and `thrust-api` must not depend, even indirectly, on a runtime or
platform crate. `scripts/check-deps.sh` checks that for exactly the crates named in its `CLEAN_CRATES` variable, and
checks `thrust-adapters-memory` separately. A new crate that has to stay portable is not covered until you add it to
that list, and CI's `deps` job runs the same script, so it does not cover that crate either. The check is also a second list: it looks only for the 17 crate names in its `FORBIDDEN` variable, in
normal dependencies, so a platform crate that is not on it passes. How the check works and how to prove it can fail
are on [Architecture gates](../testing/techniques/architecture-gates.md).

## Adding a crate

1. Create `crates/<name>/Cargo.toml`. All 13 existing crates take `edition`, `version`, `license` and `publish` from
   the workspace (`edition.workspace = true` and so on), so the licence (`AGPL-3.0-only`) and `publish = false`
   stay in one place.
2. Add the path to `members` in the root `Cargo.toml`. The list is explicit, with no glob.
3. If another crate will depend on it, add `<name> = { path = "crates/<name>" }` under `[workspace.dependencies]`
   and depend on it with `{ workspace = true }`. Eight crates are declared that way: contracts, ports, core, api and
   the four adapter crates. The two roots and `thrust-contract-tests` are not, because nothing depends on them.
   The editor crates are not either: `thrust-editor-worker` depends on `thrust-editor-doc` by path.
4. Put the crate's tests in the crate. cargo-mutants runs only the mutated package's own tests, and
   `cargo nextest run -p <crate>` selects nothing, and exits 4, for a crate with no tests of its own. The reasoning is
   Rule 5 in [Rules](../testing/rules.md).
5. If the crate has Cargo features, read [Cargo features](features.md) first.
6. A crate that builds for wasm32 as a Worker is a `cdylib`, as `thrust-worker` and `thrust-editor-worker` are, and
   is built by `worker-build` through `scripts/rs`, which puts rustup's toolchain (with the wasm32 standard
   library) ahead of Homebrew's `rustc`. The comment in `crates/thrust-worker/wrangler.toml` says so.

## Not part of the product graph

- **The editor spike.** `thrust-editor-doc` (a Rust model of the Plane editor document) and
  `thrust-editor-worker` (a bare Worker that measures its size and CPU) are Spike 0006. They are workspace members.
  Neither depends on a product crate: `thrust-editor-doc` depends on no thrust crate and `thrust-editor-worker` only
  on it. `thrust-editor-doc` keeps its own tests. The default nextest profile runs them, except its four `oracle_`
tests, which are opt-in.
- **`probes/`.** `send-shape` (which port-trait shape lets one axum router compile on native and wasm32), `sqlx-wasm`
  (whether sqlx builds for wasm32; `scripts/verify-all.sh` treats a compile failure there as the expected result)
  and `ammonia-400` (the editor sanitiser's configuration against ammonia 4.0.0). Each has its own `[workspace]`, so
  none is a member of the root workspace, and each has a `run.sh`.
- **`tools/spike-0006/`.** The TypeScript and Python oracle, corpus generators and benchmarks for the editor spike.
  How it is used is on [Differential](../testing/techniques/differential.md).
