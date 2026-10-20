---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0045: Repository and unit-of-work ports

## Context and Problem Statement

ADR-0002 (accepted) requires every platform primitive behind a port. For persistence the port must hide three very different transaction models: Postgres interactive transactions (Hyperdrive/`pg`), **D1 batch-only** (no interactive transaction; atomic multi-statement `batch()`), and DO SQLite (`transactionSync`, single-writer; **desk review only**, not a contract target — round 11) **[S: engines per ADR-0040]**. Plane's write path does multi-step work in one request (allocate sequence, insert issue, activity, M2M rows, outbox) and bulk operations bypass `save()` **[S]**. The port design decides whether the core can run on all engines.

## Decision Drivers

* Core must not know the engine (no `pg`/D1 types).
* Atomicity for (entity write + outbox row) — ADR-0046 depends on it.
* Tenant context enforced by construction (ADR-0041); named scopes (ADR-0043).
* Efficient read models for list views (avoid N+1 across the port).
* Testability with in-memory fakes.

## Considered Options

* Option 1: Per-aggregate repositories + a closure-based `UnitOfWork::run(|tx| async { … })` with interactive-transaction semantics; adapters without interactive tx emulate via a single-writer or retry-on-conflict strategy
* Option 2: Per-aggregate repositories + **command batches**: a UoW collects statements and commits atomically (`batch`), reads happen before/after; optimistic concurrency via version columns
* Option 3: CQRS split — write-side aggregate repositories (UoW) and a separate **read-model query port** (hand-tuned SQL per list/report view)
* Option 4: Generic table-gateway port (`select/insert/update` over tables), logic in use-cases

## Decision Outcome

**Decision (accepted, round 29): as built in PR #17, an explicit transaction object: `Database::begin()` returns a boxed `Tx`, the entity writes and the outbox insert go through it, `Tx::commit` consumes it, and a `Tx` dropped without a commit discards its connection instead of returning it to the pool; every repository and `Tx` method takes the FR-02 scope argument (`&Scope`, or a `SystemScope` for untenanted work) instead of bare ids. Per FR-17 and FR-02, `reviews/2026-10-19-fable-architecture-review.md` §5.**

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 3 with Option-2-compatible UoW semantics**, confidence **medium**: the write side exposes a transaction whose body may only use repositories obtained from the `tx` handle and must be expressible as *reads-then-atomic-writes* (so D1's batch model and DO's `transactionSync` can implement it; Postgres implements it with a real transaction); the read side (`IssueListQuery`, `AnalyticsQuery`, `SearchQuery`) is a separate port with adapters free to use optimal SQL. Outbox insertion is a UoW primitive (`tx.emit(event)`).)

### Consequences

* Good: engine-neutral core; explicit atomic boundary incl. outbox; fast read paths.
* Bad: constraint "reads-then-atomic-writes" forces use-case design discipline (no read-in-middle-of-tx); optimistic concurrency needed on D1; more ports to implement per adapter.
* Neutral: Postgres adapter can offer more but the contract is the minimum common denominator, enforced by contract tests.

### Confirmation

Port contract suite (ADR-0121): atomicity (failure rolls back entity + outbox), isolation (concurrent allocations), tenant predicate, scoped finders; run against the Postgres and D1 adapters as selected (DO-SQLite is desk review only); a workspace dependency check forbids adapter types in `crates/thrust-core`.

## Pros and Cons of the Options

### Option 1: Interactive UoW

* Good: natural programming model; matches Plane/Django `atomic()`.
* Bad: not implementable honestly on D1; emulation risks hidden non-atomicity.

### Option 2: Command batches

* Good: runs everywhere; explicit atomicity.
* Bad: awkward when a write depends on a prior write's result (sequence allocation → use returned value within the batch via `RETURNING`/CTE).

### Option 3: CQRS split

* Good: best performance for list views; clean write-side invariants.
* Bad: more surface area; duplication between write and read models.

### Option 4: Table gateway

* Good: tiny port; easy adapters.
* Bad: leaks relational model into core; no place for tenant/scope invariants; discourages portability of complex queries.

## Update — interview round 19 (2026-10-18)

Owner chose to **wait for the DB spike** before deciding tooling (data layer, migration tool, repository/transaction ports). Leading option from the interview (since superseded): Drizzle with repositories and a unit-of-work port.

## Update — interview round 21 (2026-10-18)

The leading option from round 19 (Drizzle) is **moot** — the backend is Rust. Candidate Rust data layers (sqlx, SeaORM, diesel, tokio-postgres + query builder) must run on `wasm32` against the chosen engine: evaluated in spike 0002. Decision still waits for the DB spike.

## Update — review fixes, round 25 (2026-10-18)

**Rust design note — the closure-based `UnitOfWork` has a real lifetime problem, not just a syntax change.** A TypeScript `uow.run(async (tx) => …)` becomes in Rust a closure returning a future that *borrows* `tx` across `.await`. That needs a higher-ranked bound (`for<'a> FnOnce(&'a mut Tx) -> BoxFuture<'a, Result<R>>`) and boxing, composes badly with `?` and with generic use-cases, and (**corrected in round 26**: the earlier claim that wasm32/workerd futures force `?Send` trait objects everywhere is wrong; spike 0001 made all port futures `Send` with `SendFuture` wrappers, ADR-0002). **Recommended shape (to confirm by spike 0002/0001): an explicit transaction object passed by `&mut`:**

```rust
#[async_trait]
pub trait UnitOfWork: Send + Sync {
    type Tx: Transaction;
    async fn begin(&self, scope: Scope) -> Result<Self::Tx, DbError>;     // Scope from the Authorizer (ADR-0051)
}
#[async_trait]
pub trait Transaction: Send {
    fn issues(&mut self) -> &mut dyn IssueRepo;      // repositories borrowed from the tx
    fn emit(&mut self, event: Event);                // outbox insert is a Tx primitive (ADR-0046)
    async fn commit(self) -> Result<(), DbError>;    // consumes the Tx; Drop without commit rolls back
}
// use-case: async fn create_issue(tx: &mut impl Transaction, cmd: CreateIssue) -> Result<Issue>
```

Postgres implements `Tx` with a real transaction. D1 (batch-only) implements the *same* trait as a **command buffer**: writes are queued on the `Tx` and sent as one `batch()` at `commit`, and reads happen before writes (the 'reads-then-atomic-writes' contract above); sequence allocation uses `UPDATE … RETURNING` (ADR-0043 B1). **DO-SQLite is dropped as a contract target** (desk review only, round 11). The decision still waits for the DB spike; the ADR stays `proposed`.

## Update — spike results, round 26 (2026-10-18)

Spike 0001 evidence for the recommended shape: the slice used an **explicit transaction object**, not a closure — `Database::begin()` returns a boxed `Tx`, `Tx::commit` consumes it, and repositories and the outbox write go through the `Tx`. Locally against Postgres (also over wrangler's Hyperdrive emulation): a rollback leaves no counter gap, a unique violation maps to `Conflict`, and an **abandoned transaction is discarded, not returned to the pool** (a mutation control: a pool that recycled a connection still inside a transaction was killed by `an_abandoned_transaction_does_not_poison_the_pool`). The closure-based `run` shape was not tried, and neither was the D1 command-buffer variant. The decision still waits for the DB spike; the ADR stays `proposed`.

## Update — review round 29 (2026-10-19)

Per FR-17 and FR-02 (`reviews/2026-10-19-fable-architecture-review.md`), this ADR is accepted as built in PR #17, with the FR-02 scope argument added. The review's §5 table records the owner's decision ("apply now" for both findings).

**Acceptance (FR-17).** The accepted shape is the one spike 0001 built (round 26 above). `Database::begin()` returns a boxed `Tx`. `allocate`, `insert_work_item`, `soft_delete_work_item` and `insert_outbox` go through the `Tx`, and `commit` and `rollback` consume it (`crates/thrust-ports/src/lib.rs:140-157`). `PgTx` implements `Drop` so that a connection still inside a transaction is discarded, not returned to the pool (`crates/thrust-adapters-postgres/src/lib.rs:387-395`). The ADR is accepted without waiting for spike 0002, so the round-25 and round-26 sentences "the decision still waits for the DB spike; the ADR stays `proposed`" no longer hold. The earlier hypothesis's closure-based `run`, separate read-model query port and D1 command buffer were not built and are not part of the decision. FR-04 adds an instance-level Durable Object SQLite cell to spike 0002; that reopens the round-11 "desk review only" status of DO-SQLite in ADR-0040 and spike 0002, not here.

**Scope argument (FR-02).** The slice's port takes bare ids. Every `Database` method takes `WorkspaceId` and `ProjectId` as plain arguments, and `insert_activity`, `undispatched_outbox`, `mark_outbox_dispatched`, `outbox_depth` and `seed` take no tenant at all (`crates/thrust-ports/src/lib.rs:92-138`). `Tx::allocate` takes the workspace (`:145`), and the Postgres adapter ignores it (`_ws`, `crates/thrust-adapters-postgres/src/lib.rs:399`). Decision:

* Every repository and `Tx` method that reads or writes tenant data takes `&Scope` instead of bare `WorkspaceId`/`ProjectId`. Only `authorize` mints a `Scope` (ADR-0051). The adapter derives `workspace_id` and `project_id` from the scope.
* Untenanted operations take a `SystemScope`, which only the composition roots and the consumer mint: the outbox relay and sweeper (`undispatched_outbox`, `mark_outbox_dispatched`, `outbox_depth`), the consumer's `insert_activity`, and `seed`.
* `Scope`'s getters (`crates/thrust-core/src/lib.rs:145-155`) are not public outside `thrust-core`.
* `membership` is the one port call that runs before a `Scope` exists, and only `authorize` calls it (`crates/thrust-core/src/lib.rs:165`). Its join on `project_members` adds `m.workspace_id`; today it joins on `project_id` and `user_id` only (`crates/thrust-adapters-postgres/src/lib.rs:189-191`).
* `project_sequences` carries `workspace_id`, which it lacks today (`crates/thrust-adapters-postgres/migrations/0001_init.sql:20-24`), and `allocate` stops ignoring the workspace.
* The two-tenant property test (ADR-0041 Confirmation) stays. Where the types allow it, a planted "ignore the tenant predicate" bug fails to compile instead of failing at runtime: a use-case that passes bare ids to a repository no longer compiles. The existing control `mutant_ignore_tenant` plants its bug in `membership` (`crates/thrust-adapters-memory/src/lib.rs:66-70`), which runs before a `Scope` exists, so it stays a runtime control that the contract suite must kill.

Implementation constraint for issue #35: `Database` and `Tx` are declared in `thrust-ports`, which does not depend on `thrust-core` (`crates/thrust-ports/Cargo.toml`), and the adapters must read the ids to build their predicates. The proof type the ports take therefore cannot be `thrust-core`'s `Scope` as it stands. Issue #35 decides where that type is declared, and how its constructor stays reserved to `authorize` while the adapters can still read the ids.

## Open Questions

* Q-0045 — granularity and transaction semantics. (answered round 29, FR-17)
* Q-0040 — engine decides whether Option 1 is viable.
* Q-0046 — outbox as UoW primitive. (answered round 29, FR-17)
* Q-0045.1 — Do we require serializable isolation anywhere beyond sequence allocation (e.g. cycle transfer, bulk reorder `sort_order`)? (answered round 29, FR-22)

## More Information

[[ADR-0002]], [[ADR-0040]], [[ADR-0041]], [[ADR-0043]], [[ADR-0046]].
