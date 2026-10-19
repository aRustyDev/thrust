---
status: proposed
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

Decision pending — see Q-0045. Leading hypothesis: **Option 3 with Option-2-compatible UoW semantics**, confidence **medium**: the write side exposes a transaction whose body may only use repositories obtained from the `tx` handle and must be expressible as *reads-then-atomic-writes* (so D1's batch model and DO's `transactionSync` can implement it; Postgres implements it with a real transaction); the read side (`IssueListQuery`, `AnalyticsQuery`, `SearchQuery`) is a separate port with adapters free to use optimal SQL. Outbox insertion is a UoW primitive (`tx.emit(event)`).

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

## Open Questions

* Q-0045 — granularity and transaction semantics.
* Q-0040 — engine decides whether Option 1 is viable.
* Q-0046 — outbox as UoW primitive.
* Q-0045.1 — Do we require serializable isolation anywhere beyond sequence allocation (e.g. cycle transfer, bulk reorder `sort_order`)?

## More Information

[[ADR-0002]], [[ADR-0040]], [[ADR-0041]], [[ADR-0043]], [[ADR-0046]].
