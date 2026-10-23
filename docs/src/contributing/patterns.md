# Patterns

How code in this repository is usually written, each with the code that shows it. These are advisory: they describe
the existing code, and a pattern here binds nobody. What is enforced is on [Standards](standards.md). Files are
cited by path and symbol, not pasted, so a pattern stays correct when its code moves.

## The patterns

**A use-case is a function of `&App` and a `Principal`.** `create_work_item` validates its input, applies the rate
limit, authorizes, runs one transaction, and relays the event after the commit. `delete_work_item` authorizes, runs a
transaction and relays. The `list_*` use-cases authorize and read, with no transaction and no event. Shown by those
functions in `crates/thrust-core/src/lib.rs`.

**Authorize in the core, and carry the proof.** `authorize` returns a `Scope`, the proof that the user may act in
that workspace and project, and the use-case passes it to every repository call. The workspace comes from the route's
slug, not from the session: a `Principal` is a user and nothing else, so one session works in every workspace its user
belongs to, and `authorize` resolves the slug, the workspace membership and the project membership in one query. It
answers 404 for a slug that names no workspace, a workspace the caller is not in, or a project outside it, and 403 for
a member of the workspace who may not touch the project, so a cross-tenant probe learns nothing. Shown by `authorize`
and `Permission::min_role` in `crates/thrust-core/src/lib.rs`.

**Scope every tenant query by the tenant key.** The adapter reads the workspace and project from the `Scope` and the
SQL filters on both: the reads and the soft delete filter on `workspace_id` and `project_id`, and sequence allocation
upserts on `(workspace_id, project_id)`, with a foreign key to `projects (workspace_id, id)` that refuses a project
outside the workspace (migration 0002). `membership` joins `workspaces` on the slug, `workspace_members` on the
caller and `projects` on that workspace, and its left join on `project_members` also requires
`m.workspace_id = p.workspace_id`. Each `Tx` write also refuses a work item or outbox
row of another tenant than its scope (`check_tenant` in `thrust-ports`). Shown by `PgDatabase` in
`crates/thrust-adapters-postgres/src/lib.rs`. The planted bug `mutant_ignore_tenant` removes the check in the memory
adapter, so that `scripts/mutants.sh` can show the contract suite catches it.

**Repository calls take a `Scope`, never ids.** Every `Database` and `Tx` method takes `&Scope` or `&SystemScope`
(`crates/thrust-ports/src/lib.rs`). The exceptions are the calls that run before a `Scope` exists: `membership`, which
only `authorize` calls, and the session calls (`session`, `touch_session`, `revoke_session`, `insert_session`), which
only `authenticate`, `revoke_session` and `seed` in `thrust-core` call. A session belongs to a user and is found by
the hash of its token, so there is no tenant key to pass (`sessions` is a directory table, ADR-0041). Both proof types are declared in `thrust-ports`, because the ports take them and the adapters
read the ids from them, and each has one constructor, `mint`:
- `Scope::mint` is called only by `authorize`.
- `SystemScope::mint` is called only by the composition roots (`thrust-server`, and `thrust-worker` for both adapter
  sets), which hand it to `App::new`. `App` keeps it private, and the outbox relay, the sweeper, the queue consumer and
  `seed` pass it to the untenanted calls (`undispatched_outbox`, `mark_outbox_dispatched`, `outbox_depth`,
  `insert_activity`, `seed`).

The fields are private, so a struct literal outside `thrust-ports` does not compile, and passing bare ids to a
repository method is a type error. The `compile_fail` doctests on `authorize` show both. The constructor itself is
`pub`, so `scripts/check-deps.sh` rejects a `mint` call anywhere else in product code; tests may mint. The reason is
the cost of one missed predicate: with about 150 tables, a repository method that forgets the tenant key is the top
security risk the threat model names (ADR-0141), and a type error is the cheapest guard. Decided in ADR-0045 (round
29, FR-02), built in issue #35.

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
`QueueError`, `BlobError`, `RateError`); `EventConsumer` returns a `String`. `From` impls turn the enums
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
