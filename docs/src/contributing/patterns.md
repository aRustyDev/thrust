# Patterns

How code in this repository is usually written, each with the code that shows it. These are advisory: they describe
the existing code, and a pattern here binds nobody. What is enforced is on [Standards](standards.md). Files are
cited by path and symbol, not pasted, so a pattern stays correct when its code moves.

## The patterns

**A use-case is a function of `&App` and a `Principal`.** `create_work_item` validates its input, applies the rate
limit, authorizes, runs one transaction, and relays the event after the commit. `delete_work_item` authorizes, runs a
transaction and relays. The `list_*` use-cases authorize and read, with no transaction and no event. Shown by those
functions in `crates/thrust-core/src/lib.rs`.

**Authorize in the core, and carry the proof.** `authorize` returns a `Scope` whose fields are private, so the only
way to hold a verified workspace, project and user is to have called it, and use-cases take the ids they pass on from
the `Scope`. Nothing in the types forces a use-case to call `authorize`, so check for it in review. It answers 404 for
a project outside the caller's workspace and 403 for a project the caller can see but may not touch, so a
cross-tenant probe learns nothing. Shown by `authorize`, `Scope` and `Permission::min_role` in the same file.

**Scope every tenant query by the tenant key.** The `Database` methods that query a project's data take a
`WorkspaceId`, and the SQL filters on it: `membership` filters on `p.workspace_id` and left-joins `project_members`
for the role, and the reads filter on `workspace_id` and `project_id`. The outbox methods and `seed` take no tenant
key. Shown by `PgDatabase` in `crates/thrust-adapters-postgres/src/lib.rs`. The planted bug
`mutant_ignore_tenant` removes the check in the memory adapter, so that `scripts/mutants.sh` can show the contract
suite catches it.

**Write the event in the transaction, relay it after.** The outbox row is inserted in the same transaction as the
entity (`Tx::insert_outbox`). `relay_after_commit` asks the `Background` port to run `dispatch`, which publishes the
event through `Queue::send` and marks the row dispatched, and it swallows a failure on purpose. `sweep_outbox`
publishes up to `limit` undispatched rows per call, oldest first (the roots pass 100). The consumer is idempotent:
`insert_activity` returns `false` for an event id it has seen, and the `activity` table has a unique `event_id`.
Shown by `dispatch`, `sweep_outbox` and `process_event` in `thrust-core`.

**Roll back explicitly, and never return a mid-transaction connection to the pool.** `create_work_item` calls
`tx.rollback()` on error. `PgTx` implements `Drop`, so a transaction abandoned without a commit (a cancelled request)
makes the connection `discard` itself, and `PoolConnector` removes it from the pool instead of recycling it. The
planted bug `mutant_discard_noop` breaks exactly this.

**Time, ids and background work are ports.** Core calls `Clock`, `IdGen` and `Background`, so it never reads the
clock or spawns a task itself, and a test can supply each. Shown by `App.ports` and `event()` in `thrust-core`.

**Every port future is `Send`.** The one shared router needs `Send` handlers on both targets, so the traits use plain
`#[async_trait]`, and the Cloudflare adapters wrap their JavaScript-backed futures in `worker::send::SendFuture`.
Shown by `crates/thrust-ports/src/lib.rs` and `crates/thrust-adapters-cloudflare/src/lib.rs`.

**One error type per layer, mapped at the boundary.** Each fallible port has its own error enum (`DbError`,
`QueueError`, `BlobError`, `CacheError`, `RateError`); `EventConsumer` returns a `String`. `From` impls turn the enums
into `CoreError`, and `ApiError` turns `CoreError` into a status and a code. An internal error's real cause travels in an
`InternalErrorDetail` response extension for the root to log, and the body says only `internal error`. Shown by
`crates/thrust-core/src/lib.rs` and `crates/thrust-api/src/lib.rs`.

**Write the SQL once, and vary how the connection is obtained.** `PgDatabase` is written against
`tokio_postgres::Client`, and a `Connector` supplies the client: a pool on the server, a Hyperdrive socket on the
Worker. Run statements through the `Pg` wrapper, not `Client::query`: its comment records that the default prepared
statement flow was unreliable through Hyperdrive and that one typed pipeline per statement was not.

**Keep configuration out of the layers below the roots.** `thrust-server` and `thrust-worker` read the environment
for the application's configuration and hand `CoreConfig` and `ApiConfig` to the layers below. Core, the router and the
ports read nothing from the environment.

## Recipes

**Add a route.**
1. Write the use-case in `thrust-core`, starting with `authorize`.
2. Add a handler in `crates/thrust-api/src/lib.rs`. It takes `Authed`, the `App` extension and the path and body,
   calls the use-case and returns `ApiResult`.
3. Register it in `router()`. Both roots serve that router, so nothing platform-specific is needed.
4. Add a contract test that pairs the allowed case with its rejected neighbour, and raise the pass's `EXPECT_*` count
   in `scripts/contract.sh`. [Contract suite](../testing/techniques/contract-suite.md) says how.
5. Run `scripts/check-deps.sh`.

**Add a port.**
1. Add the trait, with `Send` futures, and its error enum to `crates/thrust-ports/src/lib.rs`.
2. Add a field to `Ports` in `thrust-core`.
3. Implement it in `thrust-adapters-memory` first: its crate documentation calls it the reference implementation for
   the contract suite. It has no adapter for `Clock`, `IdGen` or `Background`, so skip this step for a port like those.
4. Implement it in each platform adapter that has the capability.
5. Build it into `Ports` in all three places that construct one: `crates/thrust-server/src/main.rs`, the
   `cloudflare` arm of `build_app` in `crates/thrust-worker/src/lib.rs`, and `crates/thrust-worker/src/memory.rs`.
6. Add a contract test, and run `scripts/check-deps.sh`.

**Add an adapter for an existing port.** Implement the trait in the adapter crate for that platform, construct it in
the root that uses it, and run the contract suite against it: the suite is the same for every target. For a new
way of reaching Postgres, implement `Connector` and leave the SQL alone.

**Plant a bug to prove a test can fail.** Add a `mutant_*` feature with the `cfg` pair, forward it through
`thrust-server` if the contract script must reach it, and add it to the list in `scripts/mutants.sh`. The steps and
the reasoning are on [Mutation](../testing/techniques/mutation.md), and the feature mechanics on
[Cargo features](features.md).
