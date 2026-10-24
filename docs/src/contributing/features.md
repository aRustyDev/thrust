# Cargo features

Seven of the 13 workspace crates declare Cargo features, and six declare none. The adapter sets are not among the
features: there is no `cloudflare` or `selfhosted` feature, even though two crate descriptions use those words. This
page lists every feature of the workspace members, how the build scripts switch them on, and what the existing ones
show about how to add another. The three probe crates are outside the workspace and declare features of their own
(`unsend` and `cfg-switch` in `probes/send-shape`, `tokio` in `probes/sqlx-wasm`); this page does not cover them.

## Every feature

| Crate | Feature | Default | What it does |
|---|---|---|---|
| `thrust-worker` | `memory` | on | Compiles the in-isolate memory adapter set, and makes it the default value of `THRUST_ADAPTERS`. With the feature off, the Worker accepts only `cloudflare` |
| `thrust-adapters-postgres` | `native` | off | Turns on `tokio-postgres/runtime` and `tokio`. The `migrate` binary requires it (`required-features`), and `thrust-adapters-selfhosted` depends on the crate with it. Never enable it for wasm32 |
| `thrust-adapters-memory` | `mutant_ignore_tenant`, `mutant_dup_sequence`, `mutant_no_activity_dedupe` | off | Planted bugs: forget the tenant predicate, hand out the same sequence number every time, skip the activity de-duplication |
| `thrust-adapters-selfhosted` | `mutant_discard_noop` | off | Planted bug: hand a connection that is still inside an abandoned transaction back to the pool |
| `thrust-adapters-sqlite` | `native` | off | Turns on rusqlite (bundled SQLite, compiled from C): `NativeSql`, the native twin. `thrust-server` enables it; the Worker must not (`scripts/check-deps.sh` checks `rusqlite` and `libsqlite3-sys` are absent from the Worker graph) |
| `thrust-adapters-sqlite` | `mutant_dup_sequence`, `mutant_no_gate`, `mutant_gate_leak`, `mutant_non_atomic_commit`, `mutant_broken_migration` | off | Planted bugs: always sequence 1; transactions not serialised; an abandoned transaction never reopens the gate; a commit applied statement by statement; a migration that fails halfway (a demonstration build, not a kill control) |
| `thrust-adapters-memory` | `mutant_search_case_sensitive` | off | Planted bug: search without folding case |
| `thrust-worker` | `mutant_broken_migration` | off | Forwards to `thrust-adapters-sqlite`: an object whose migration fails, to see what the platform does |
| `thrust-server` | the `mutant_*` names (memory's, and `mutant_dup_sequence` and `mutant_no_gate` also for SQLite) | off | Forwards each one to the adapter that has it, so a build of the server can switch a planted bug on |
| `thrust-editor-doc` | `sanitize` | on | Pulls in the ammonia sanitiser (and html5ever). It is split out so the size of the conversions alone can be measured |
| `thrust-editor-worker` | `sanitize` | on | Forwards to `thrust-editor-doc/sanitize` |

These crates declare no features: `thrust-contracts`, `thrust-ports`, `thrust-core`, `thrust-api`,
`thrust-adapters-cloudflare` and `thrust-contract-tests`.

## Adapter sets are not features

ADR-0007 (accepted, `.agents/plans/contrail/adrs/architecture/0007-adapter-selection-mechanism.md`) chooses the
adapter set at compile time, in the composition root. The tree does that without a feature of that name:

- **The Worker.** The Cloudflare set is the crate's dependency list and is always compiled. `memory` is its only
  feature. `THRUST_ADAPTERS` then chooses among the sets that were compiled in: `cloudflare`, or `memory` when the
  feature is on.
- **The server.** It always compiles the memory adapters and the Postgres adapter, and `THRUST_ADAPTERS` picks
  `memory` or `postgres` when the process starts. No feature is involved.

The environment variables are on [Infrastructure](../architecture/infra.md).

## How the scripts switch features on

The two `scripts/mutants.sh` bullets are `[verified 2026-10-19 c6376a6]`: the script ran, and all four planted bugs
were killed. The Worker bundle and `native` bullets are `[derived]` from the scripts and not run.

- **The Cloudflare-only Worker bundle:** the `[build]` command in `crates/thrust-worker/wrangler.toml` runs
  `worker-build --release . --no-default-features`, which drops `memory`. `wrangler.memory.toml` builds with the
  defaults.
- **A planted bug in the server:** `scripts/mutants.sh` sets `THRUST_SERVER_FEATURES` to one `mutant_*` name and runs
  `scripts/contract.sh native-memory main`. The contract script passes the value to `cargo build -p thrust-server
  --features`.
- **The planted Postgres bug:** `scripts/mutants.sh` runs
  `cargo test -p thrust-adapters-selfhosted --features mutant_discard_noop --test pg_adapter -- an_abandoned_transaction`.
- **`native`:** `thrust-adapters-selfhosted` enables it through its dependency on the Postgres crate, and
  `scripts/contract.sh` builds the `migrate` binary with `--features native`.

## What the existing features show

This is practice, not a rule.

- **A planted bug is a feature pair.** Each one is a `#[cfg(feature = "mutant_...")]` branch and a
  `#[cfg(not(feature = "mutant_..."))]` branch around the line it breaks, so the default build contains only the
  correct code. The memory adapter's Cargo comment says they are never enabled outside `scripts/mutants.sh`, and
`scripts/contract.sh` says the same of `THRUST_SERVER_FEATURES`. A bug that the contract
  suite must catch is forwarded through the root (as `thrust-server` does), because the contract script builds the
  root, not the adapter. The Postgres one is exercised by the adapter's own test, so it is not forwarded.
- **A feature that adds a dependency names it with `dep:`**, as `memory`, `native` and `sanitize` do.
- **Platform separation is checked by the graph, not by features.** `native` must stay out of the wasm32 build.
  `scripts/check-deps.sh` asks `cargo tree` whether a native-only crate is in the Worker's wasm32 graph, built with
  `--no-default-features`, whichever feature put it there. It checks the Cloudflare set only, not the `memory` set.
- **A default feature of `thrust-worker` is left out of the production bundle.** The `wrangler.toml` build uses
  `--no-default-features`, which drops every default feature, not only `memory`. A feature added to the Worker's
  defaults would reach `wrangler.memory.toml`'s build and not the bundle, unless that command names it.
