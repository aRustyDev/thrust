---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0043: IDs, per-project sequencing, and soft delete

## Context and Problem Statement

Three persistence conventions in Plane leak into API behaviour and must be decided deliberately **[S]**:

1. **Primary keys** are UUID v4 (`BaseModel`), exposed in every API payload.
2. **`sequence_id`** is a per-project running integer (the "PLATF-7" human key). `Issue.save()` computes it under a Postgres advisory lock; `IssueSequence` records it; bulk operations bypass `save()`.
3. **Soft delete** (`deleted_at`) is universal; uniqueness is enforced with **partial unique constraints** (`WHERE deleted_at IS NULL`); two-tier managers hide triage/archived/draft rows (`Issue.issue_objects` vs `Issue.objects`).

The new backend must keep API-visible semantics (ADR-0031) while not depending on Postgres-only advisory locks if the DB choice (ADR-0040) is open.

## Decision Drivers

* ID format compatibility with `/api/v1` clients (UUIDs retained).
* Gap-free-ish, collision-free `sequence_id` under concurrency, portable across DB engines.
* Bulk/import paths must not bypass invariants (the `save()` bypass gotcha).
* Unique-name semantics must survive soft delete and re-creation.
* "Hidden-by-default" visibility must be hard to forget (security: drafts/triage leaking).

## Considered Options

### A. Primary keys
* A1: Keep UUID v4 for all rows
* A2: UUID v7 (time-ordered) for new rows
* A3: ULID/integer surrogate internally + UUID external id

### B. `sequence_id` allocation
* B1: Counter row per project (`project_sequences(project_id, next)`), allocated with one atomic `UPDATE … RETURNING` inside the creating transaction
* B2: `MAX(sequence_id)+1` under an engine lock (Plane's approach; Postgres advisory lock)
* B3: Single-writer allocator (DO per project/workspace serializes allocation)

### C. Soft delete
* C1: Keep `deleted_at` + partial unique indexes; deletion handled in repositories with named scopes
* C2: Hard delete + tombstone/archive table
* C3: `deleted_at` + application-level uniqueness checks (no partial indexes)

## Decision Outcome

**Decision (accepted, round 29): A2, B1 and C1, as built in PR #17 (B1 counter row, `sort_order` in the same critical section, partial unique indexes), with the FR-22 allocation order. Per FR-17 and FR-22, `reviews/2026-10-19-fable-architecture-review.md` §5.**

The accepted options as first written (confidence as recorded before round 29; the allocation order is set in the round-29 note):

* **A2** — UUID v7 for new rows (medium): same wire format, better index locality; no client-visible change.
* **B1** — counter row with atomic `UPDATE … RETURNING` in the same transaction (medium-high): works on Postgres and SQLite/D1 (single statement); no advisory lock; the allocation joins the create's transaction/batch. **It must replicate Plane's `MAX(IssueSequence.sequence)+1` semantics (verified against `db/models/issue.py`):** Plane computes the next number from the `IssueSequence` table (FK `SET_NULL` on the issue), so **numbers are never reused after an issue is deleted or moved** — the counter row must be monotonic and never decrement. In the same locked block Plane also allocates **`sort_order`** (largest in the state + 10,000) and `description_stripped`; the allocator must cover `sort_order` too (R-F2). Gaps on rollback are acceptable if documented (Plane's own behaviour on failed save is similar [S, verify]).
* **C1** — keep `deleted_at` + partial unique indexes, with repository **named scopes** replacing two-tier managers: `visible()` (default; excludes triage, archived, **project-archived**, draft and deleted — Plane's `IssueManager` hides four classes: triage state group, `archived_at`, project archived, `is_draft`), `withHidden()`, `withDeleted()` (medium-high). The default scope is applied inside the repository so call sites cannot forget it.

### Consequences

* Good: API data unchanged; one allocation mechanism for all engines; hidden-row leakage becomes a repository-layer invariant.
* Bad: counter row is a hot row per project (contention on very busy projects — measured in ADR-0040 spike S1); UUIDv7 leaks creation time ordering (acceptable; already exposed via `created_at`).
* Neutral: bulk-create paths must call the same allocator (provide `allocateSequenceRange(project, n)`); outbox emission must also be explicit on bulk paths (ADR-0046).

### Confirmation

Spike S1/S2 (ADR-0040); contract test "concurrent creates never duplicate a sequence_id"; contract test "re-create a soft-deleted unique name succeeds; two live duplicates fail"; static check that repositories expose only scoped finders.

## Pros and Cons of the Options

### A1 / A2 / A3
* A1: simplest, zero change; random-key insertion fragmentation on large tables.
* A2: locality and sortability; mixed-version IDs; needs v7 generator on all runtimes.
* A3: double ID system; API complexity; no benefit given UUID compat requirement.

### B1 / B2 / B3
* B1: portable, simple; hot row; gaps on rollback.
* B2: matches Plane exactly; requires engine-specific locking (not available on D1; DO needs none); slower under load.
* B3: serialization for free; couples allocation to a DO and breaks portability unless emulated.

### C1 / C2 / C3
* C1: preserves Plane semantics and history/audit chains (`created_by`); index bloat from dead rows; hard delete jobs needed (Plane has daily `hard_delete` **[S]**).
* C2: smaller hot tables; loses in-place restore; migration complexity; breaks audit linkage.
* C3: engine-agnostic but racy without constraints.

## Update — interview round 16 (2026-10-17)

Moving a work item gives it a **new sequence key and keeps the old key as an alias** so links survive (Q-0043.1). Other items open.

## Update — interview round 20 (2026-10-18)

Keep the daily hard-delete cadence with per-table retention, **30 days default** (Q-0043.2).

## Update — review fixes, round 25 (2026-10-18)

Corrections from the review against `apps/api/plane/db/models/issue.py:179-235`: (1) `sequence_id` is `MAX(IssueSequence.sequence)+1`, not `MAX(Issue.sequence_id)+1`, so numbers are never reused — B1's counter row must behave identically; (2) the same locked block allocates `sort_order` (R-F2 and the spike must cover it); (3) the hidden-row set has **four** classes, not three. **Retention (round 20, deviation recorded):** the default hard-delete retention is **30 days**, a **deliberate deviation from Plane's 60** (`HARD_DELETE_AFTER_DAYS`, `settings/common.py:404`); and because nothing would otherwise purge soft-deleted rows, the **hard-delete cleanup (F-D10) is a P0 deliverable**. Plane's `hard_delete` covers an explicit list of ~18 models, not every table (`bgtasks/deletion_task.py:114-175`). Import-era wording removed (Q-G3: start fresh).

## Update — spike results, round 26 (2026-10-18)

Spike 0001 evidence, Postgres only (spike 0002 still runs the 10,000-create, 50-writer test): option **B1, a per-project counter row upserted in the same transaction, also allocating `sort_order`**, worked. 20 concurrent creates over HTTP and 24 concurrent allocations at the adapter level (real Postgres through testcontainers) gave unique, contiguous numbers; a rollback leaves no gap; soft-deleting an item frees its `external_id` (partial unique index) but never its sequence number. A duplicate-sequence mutant is killed by four tests. Concurrency cost: create p95 at 8 clients was 140 ms against 27 ms at one client on the local Cloudflare-set Worker, which I read as the hot counter row serialising creates in one project — not isolated, treat as a prediction for Stage 2. **B3 (a Durable Object as the allocator) was not tried.**

## Update — review round 29 (2026-10-19)

Per FR-22 and FR-08 (4) (`reviews/2026-10-19-fable-architecture-review.md`), the sequence number is allocated in its own short transaction before the create's transaction, or by an `UPDATE … RETURNING` as the last statement before commit, so that the counter-row lock is held for one round trip. Today the counter upsert (`crates/thrust-adapters-postgres/src/lib.rs:399-414`) is the first statement of the create transaction (`crates/thrust-core/src/lib.rs:250-295`: allocate, insert, outbox row, commit), so the row lock is held across the insert, the outbox write and the commit, and each create in a project waits for the one before it. `research/0004` §10.3 measured create p95 559 ms at 8 clients against 296 ms at 1, attributed to the hot row but not isolated. Plane's advisory lock serialises creates the same way, so this is parity, not a regression; advisory locks are unsupported through Hyperdrive (FR-04, ADR-0002 round 29), so B1 stays the mechanism. Gaps on failure are accepted: a create that fails after its allocation committed loses that number, which the B1 bullet already allows, and the counter stays monotonic. This gives up the round-26 property "a rollback leaves no gap" on the create path. `crates/thrust-adapters-selfhosted/tests/pg_adapter.rs:134` asserts that property at the adapter level, so the change that implements this order must restate that test and say why in its commit message (`AGENTS.md`, Testing rule 3). Confirm with spike 0002's 50-writer test (the 10,000-create run named in the round-26 note) and re-measure create p95 at 8 clients (FR-08).

Per FR-17, A2, B1 and C1 are accepted as built in PR #17, and Q-0043, Q-0040.2 (B1) and Q-0040.3 (partial unique indexes: yes on Postgres; D1 is moot) are answered. As built: **A2**, UUID v7 from both `IdGen` adapters (`crates/thrust-adapters-selfhosted/src/lib.rs:19-20`, `crates/thrust-adapters-cloudflare/src/lib.rs:31-38`). **B1**, the `project_sequences` counter row, monotonic and never decremented, with `sort_order` allocated in the same statement (`crates/thrust-adapters-postgres/migrations/0001_init.sql:18-24`), and a unique index on `(project_id, sequence_id)` across live and deleted rows (line 44); the statement is an upsert (`INSERT … ON CONFLICT DO UPDATE … RETURNING`), so it also creates the row on a project's first create. **C1**, a partial unique index on the live external key (lines 46-49) and two scopes applied inside the adapter's queries, `Visibility::Visible` (excludes deleted, archived and draft rows) and `Visibility::WithDeleted` (`crates/thrust-contracts/src/lib.rs:110-115`). Not built yet, as expected at slice size (review §4): the triage-state and project-archived hidden classes, `withHidden()`, and the bulk `allocateSequenceRange`.

## Update — FR-22 implemented, issue #37 (2026-10-24)

FR-22's allocation order is built in the Postgres adapter (thrust PR #77): `PgTx` checks its connection out at `begin` and opens no transaction until its first write, so `PgTx::allocate` (`crates/thrust-adapters-postgres/src/lib.rs`) runs the counter upsert as one statement in autocommit before the create's transaction, and the counter row is locked for one round trip, not through the insert, the outbox insert and the commit (`create_work_item` in `crates/thrust-core/src/lib.rs`). A second `allocate` in a transaction that is already open takes its number on another connection. A number taken by a create that then fails or is abandoned is not given back, as the round-29 note accepted.

The three Postgres adapter tests that asserted "a rollback leaves no gap" were restated, with the reason in the commit message: `rollback_is_atomic_and_unique_violation_is_a_conflict` (`pg_adapter.rs`), `an_abandoned_transaction_does_not_poison_the_pool` and the generated `model_postgres_adapter_matches_the_reference_model`. The SQLite adapter writes its counter at commit and keeps the no-gap property; the memory adapter has always had gaps. Locations in the round-29 note above (`lib.rs:399-414`, `lib.rs:250-295`, `pg_adapter.rs:134`) are the old ones.

**Evidence, and what it is not:** the 50-writer test passes on every engine, locally and on the deployed `worker-do`; the same test against `mutant_dup_sequence` fails. Create p95 at c=1 and c=8 on `native-pg`, before then after, with a 5 ms one-way delay proxy (RTT 10 ms): c=8 went from 344 / 360 / 358 ms to 100 / 97 / 105 ms and c=1 stayed about 93 ms, so the c=8 cost was the hot row held across round trips, as the note said. On loopback no difference is visible (the lock is held for microseconds). The proxy is a stand-in, and the platform result is smaller. **On the platform** (the deployed `worker-pg` cell, 2026-10-24: Hyperdrive, the VPC service and the tunnel, placed in `us-east-1`, one laptop in the origin's region, `bench` n=200), the pre-FR-22 shape was rebuilt for one deployment and run the same hour as the committed order (`research/0011` section 9.1): **at c=1 there is no difference** (create p50 about 123 ms both ways); **at c=8 the p50 is about 15 percent lower** (mean 210 ms before, 178 ms after; every FR-22 run below every pre-FR-22 run) and the c=8 to c=1 ratio falls from 1.7 to 1.4; **the p95 and p99 are not distinguishable** (332-727 ms before, 300-558 ms after). The proxy's near-flat c=8 does not reproduce: c=8 stays 40 percent above c=1, so the hot row was a real cost but not the only one (untested candidates: the per-user rate-limiter object, a Hyperdrive pool of 8 for two connections per create, the client link). Next to stage 2's 296 and 559 ms p95 the cell now reads 145-198 ms at c=1 and 300-558 ms at c=8 with the order, and 157-243 and 332-727 ms without it: most of the change since stage 2 is the day, the build and the vantage, not FR-22.

**Confirmed and revised, on the platform:** the order is correct and harmless there, and the price this ADR accepted (a number lost to a failed create) buys a p50 improvement of about 15 percent at c=8 on this vantage and nothing at c=1; the benefit is smaller than the delay proxy suggested. The order is not revised; whether that benefit is worth the gaps is the owner's to weigh, and this note changes neither the decision nor the accepted price.

For `worker-do`, allocation is serialised inside the object: the counter is read by `allocate` and written by the commit, and a one-transaction-at-a-time gate keeps another transaction from moving it in between (ADR-0045 note).

## Open Questions

* Q-0043 — approve A/B/C choices. (answered round 29, FR-17, FR-22)
* Q-0040.2 — portable sequence allocation (this ADR proposes B1). (answered round 29, FR-17, FR-22)
* Q-0040.3 — partial unique index availability. (answered round 29, FR-07)
* Q-0043.1 — Moving a work item between projects: re-sequence (new key) or keep an alias? (Plane behavior to verify.)
* Q-0043.2 — Retention: keep the daily `hard_delete` cadence, and which tables are covered?

## More Information

[[ADR-0040]], [[ADR-0042]], [[ADR-0045]], [[ADR-0046]]. `apps/api/plane/db/models/{issue,base}.py`, `db/mixins.py`; research `woven/plan:.claude/plans/plane-fork/research/backend-data-model.md` §§2,5.
