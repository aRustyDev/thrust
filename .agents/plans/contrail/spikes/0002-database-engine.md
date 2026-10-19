# Spike 0002 — Database engine (Postgres + Hyperdrive vs D1; DO-SQLite desk review)

**Gates:** ADR-0040, 0041, 0042, 0043, 0044, 0045, 0083.
**Questions:** Q-0040, Q-0040.1–.5, Q-0041, Q-0044, Q-0045.1, Q-0083.1, Q-0043.
**Timebox:** about 3.5 working days (round 27: Postgres only). **Depends on:** spike 0001 (Rust Worker + DB driver path) and the CNPG Postgres with its tunnel and Workers VPC service (ADR-0094, bead `plane-byd.4`). **Owner:** TBD.

## Question
Which engine meets the P0 workload from Rust on both Workers and native, and which Rust data-access layer (sqlx,
SeaORM, diesel, tokio-postgres + query builder) works on `wasm32` against it?

## Candidates
- **A. Postgres via Hyperdrive** (rust-postgres/tokio-postgres is the Rust driver Cloudflare lists as supported). Round 27: the instance is CNPG in woven-o11y, reached through a Workers VPC service and a new tunnel (ADR-0094).
- **B. D1** (workers-rs D1 support is marked alpha — verify current state). Round 27: **desk review only**, no prototype.
- **C. DO-SQLite per workspace** — desk review only; reject unless cross-workspace needs are nil (Q-0040.4).

## Method (A prototyped, B desk-reviewed since round 27; seed ≈100k work items, 50 users)
1. Schema subset: workspace, project, state, label, issue (+ assignees/labels joins), outbox, activity, with `deleted_at`
   soft delete and partial unique indexes.
2. **sequence_id** allocation under concurrent creates (10k creates from ≥50 parallel writers, one project): monotonic, no duplicates, never reused after delete/move (Plane: `MAX(IssueSequence.sequence)+1`), `sort_order` allocated in the same critical section (R-F2). Candidates: per-project counter row with `UPDATE … RETURNING`, advisory lock (PG only), DO allocator.
3. Partial unique indexes and soft-delete uniqueness behave as Plane's.
4. **FTS + substring parity** (ADR-0083, round 25): Postgres `tsvector`/GIN + `pg_trgm` vs D1 FTS5 (+ `trigram` tokenizer) — Plane's search is `icontains`; query by title/description, substring/prefix parity, ranking, 1-minute freshness.
5. Transactions: multi-statement atomicity for outbox + data (D1 batch vs PG tx); isolation needed for cycle transfer and
   bulk `sort_order` (Q-0045.1).
6. Rust data layer on `wasm32`: which of sqlx / SeaORM / diesel / raw driver compile and run in a Worker; migration tool.
7. Latency: p95 for list-issues (grouped, paginated), create, update at 100k items from a Worker (Smart Placement
   on/off, Hyperdrive caching on/off); region choice (Q-0040.5).
8. Tenant isolation hypothesis check (ADR-0041): row-level `workspace_id` predicate enforced in repositories; cost of RLS on PG.
9. Cost projection at sizing target, of the Cloudflare side; report the Postgres host separately, it is outside the envelope (round 27) (shared dev DB: define schema-per-PR ownership, cleanup and migration locking).

## Exit criteria
Engine passes if: no duplicate `sequence_id` in 10k creates from ≥50 parallel writers; partial unique indexes enforced; FTS meets
1-minute freshness; list p95 < 300 ms (leaves headroom to the 500 ms SLO); a Rust data layer runs on both targets;
projected cost within envelope. If both pass, prefer Postgres (closest to Plane's schema and the self-hosted twin).

## Deliverables
Decision in ADR-0040 (+ 0041/0042/0043/0044/0045 updated); benchmark data in `research/`; Q-0083.1 closed.
