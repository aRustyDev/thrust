---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0042: Schema source of truth and migrations (frozen Plane end-state baseline; no data import)

## Context and Problem Statement

Plane's schema is defined in Django models and evolved by **121 migrations** (`db/migrations/0001_initial` → `0121_alter_estimate_type`, plus the separate `license` app migrations), including `RunPython` data migrations and hand-named ones (`0074_deploy_board_and_project_issues`, `0107_migrate_filters_to_rich_filters`) **[S]**. The new backend has no Django. We need (a) a new schema source of truth, (b) a migration tool that runs on the chosen DB(s), (c) a way to adopt Plane's *end-state* without replaying 121 migrations. (A one-time import of existing data, originally item (d), was **dropped** — Q-G3: start fresh.) Convention to preserve: soft-delete partial unique constraints declared with every unique **[S]**.

## Decision Drivers

* Migration tool must support the DB(s) from ADR-0040 (Postgres and/or SQLite dialect) and both adapter sets.
* Reviewable, forward-only, additive migrations (Open-EE's stated policy **[O]**).
* Type-safe access in the query layer (ADR-0044).
* ~~Reproducible import from a live Plane Postgres~~ — dropped (Q-G3).
* Multi-tenant/per-workspace fan-out (ADR-0041) if chosen.

## Considered Options

* Option 1: ~~TypeScript schema as source (Drizzle)~~ — **superseded by the Rust decision (ADR-0006)**; Rust analogue: a Rust-defined schema (e.g. SeaORM entities / diesel `schema.rs`) with generated migrations committed
* Option 2: Hand-written, reviewed SQL migrations (forward-only) as source of truth + generated Rust types (introspection/codegen, e.g. `sqlx` offline metadata or `sea-orm-cli generate entity`)
* Option 3: Declarative schema tool (Atlas / similar) with plan-based diffs
* Option 4: Keep Django solely as a migration/schema tool (run `manage.py migrate`), new backend reads the resulting DB

## Decision Outcome

**Decision (accepted, round 29): as built in PR #17, hand-written SQL migration files are the schema source of truth, applied by the `migrate` binary; every migration is expand/contract; no ORM and no codegen of Rust types from the schema. Per FR-17, `reviews/2026-10-19-fable-architecture-review.md` §5.**

This is **Option 2** without its codegen half, chosen because it is dialect-explicit (partial indexes, `RETURNING`), engine-agnostic, and makes the Plane end-state adoption a literal first migration. The Drizzle-based Option 1 is superseded by the Rust backend; its Rust analogue needed an ORM that owns the schema, and ADR-0044 (accepted round 29) picks none.

### Adopting the end-state (applies to any option)

1. Stand up a clean Plane at the fork's base version; run all 121 migrations; `pg_dump --schema-only` → reviewed baseline migration `0001_baseline`.
2. **Exclude Django-owned/infra tables**: `django_migrations`, `django_content_type`, `django_session`, `auth_*`, `django_celery_beat_*`, `django_celery_results` if present. Keep `license` app tables (`Instance`, `InstanceConfiguration`, `InstanceAdmin`) in the directory namespace.
3. Normalize: drop Django-specific `*_pkey`/sequence naming only if the importer does not depend on it; replace Postgres-only types if a SQLite adapter is retained (ADR-0040).
4. A **schema-parity test** compares the baseline against a fresh `pg_dump` of upstream at the pinned version (guards drift when re-baselining on upstream releases, Q-G1/Q-G2).

### ~~One-time data import (Q-G3)~~ — **void (start fresh; round 5)**

The bullets below are kept for history only; nothing is imported, so no importer, checksum or fixture-dump work is planned.

* Source: logical copy (`COPY`/ETL), **preserving UUID primary keys** and `sequence_id` values, `created_at/updated_at`, `created_by` chains, `deleted_at` rows (policy: import soft-deleted rows or drop? — decide in Q-0042.2).
* Not imported: sessions, Celery state, magic-code Redis keys. API tokens: import and re-hash (ADR-0052).
* Files: copy MinIO/S3 objects to R2 (same keys) and rewrite storage references per ADR-0047.
* Verification: per-table row counts + checksum of key columns; replay a sample of API v1 reads against both systems (ADR-0122).
* Collaboration state: `description_binary` (Yjs) imported byte-for-byte (editor format unchanged, Q-0012).

### Consequences

* Good: clear provenance; import is mechanical.
* Bad: re-baselining on upstream schema changes is manual work; losing Django's `RunPython` data-migration history (end-state only — acceptable, history is not needed).

### Confirmation

CI job: apply all migrations to an empty DB on every supported engine and run the schema-parity test against the pinned Plane dump (the import dry-run is dropped with the import).

## Pros and Cons of the Options

### Option 1: Drizzle schema + generated migrations (superseded — TypeScript; kept for history)

* Good, because single TS source, strong types, runs on Workers.
* Bad, because separate schema definitions per dialect (pg vs sqlite) if both are retained; generated SQL can hide partial-index subtleties; vendor lock to Drizzle's migration journal.

### Option 2: Hand-written SQL migrations + codegen

* Good, because explicit and reviewable; any engine; easy end-state adoption.
* Bad, because more manual effort; types generated post-hoc; two dialect files if both retained.

### Option 3: Declarative (Atlas etc.)

* Good, because drift detection and safe diffs/linting.
* Bad, because extra tool/binary in CI; less friendly to D1/DO-SQLite targets; learning curve.

### Option 4: Django as migration tool

* Good, because zero schema port; guaranteed parity.
* Bad, because keeps Python/Django in the toolchain and ties schema evolution to upstream models, contradicting ADR-0001's "rewrite backend"; unusable on D1/DO.

## Update — interview round 5 (2026-10-17)

**Q-G3 answered: start fresh — no one-time import.** The "import the end-state of Plane's 122 Django migrations" and "data import" threads in this ADR are **no longer required**. The schema may be designed for the new domain model; it only needs to stay close enough to Plane's entity shapes for the *retained frontend* and the wire-compatible `/api/v1` (ADR-0031). Q-0042.2 (import soft-deleted rows) is moot. Greenfield migration tooling choice (Q-0042) is simplified: no Django-baseline reconciliation, no data-fidelity tests against a Plane dump.

## Update — interview round 19 (2026-10-18)

Owner chose to **wait for the DB spike** before deciding tooling (Drizzle/Kysely, migration tool, repository/transaction ports). Leading option from the interview: Drizzle with repositories and a unit-of-work port.

## Update — interview round 20 (2026-10-18)

Schema baseline: **freeze at the pinned Plane version; nothing is imported** (Q-0042.1, Q-0042.2). (The earlier phrase 'soft-deleted rows included' was meaningless given start-fresh and is withdrawn in round 25.)

## Update — interview round 21 (2026-10-18)

The leading option from round 19 (Drizzle) is **moot** — the backend is Rust. Candidate Rust data layers (sqlx, SeaORM, diesel, tokio-postgres + query builder) must run on `wasm32` against the chosen engine: evaluated in spike 0002. Decision still waits for the DB spike.

## Update — review fixes, round 25 (2026-10-18)

Count corrected to **121 migrations** (`0001`–`0121`). Import-era text (end-state import, per-table checksums, API-token re-hashing, byte-for-byte Yjs import, the anonymised-dump dry-run, and the 'soft-deleted rows included' phrase) is struck or withdrawn because Q-G3 is answered 'start fresh'. Drizzle (TypeScript) is superseded by the Rust backend; Rust migration/schema tooling candidates are evaluated in spike 0002 and the decision still waits for it. The ADR stays `proposed`.

## Update — review round 29 (2026-10-19)

Per FR-17 (`reviews/2026-10-19-fable-architecture-review.md`), this ADR is accepted as built in PR #17 (owner disposition in review §5); spike 0002 no longer decides tooling (FR-16 drops its data-layer step). As built: the schema is plain SQL in `crates/thrust-adapters-postgres/migrations/` (one file today, `0001_init.sql`), embedded in the adapter as `MIGRATION_0001` and applied with `batch_execute` (`crates/thrust-adapters-postgres/src/lib.rs:20-24`) by the native-only `migrate` binary (`crates/thrust-adapters-postgres/src/bin/migrate.rs`; `crates/thrust-adapters-postgres/Cargo.toml:14-17`), which reads `DATABASE_URL` from the environment. `scripts/contract.sh:120-122` applies the schema through that binary, and neither the server nor the Worker migrates at start (`docs/src/architecture/infra.md:128-132`). Rows are mapped by hand, and type safety comes from the typed parameters of ADR-0044's `Pg` wrapper, not from generated types; so Option 2's codegen half (`sqlx` offline metadata, `sea-orm-cli`) and Option 1's Rust analogue (SeaORM entities, diesel `schema.rs`) are not adopted, and neither are Options 3 and 4. Every migration follows the expand/contract rule that ADR-0110 already sets for deploys.

Not built yet: `0001_init.sql` is the spike's schema, not the product schema (its line 1); the binary applies that one file through `IF NOT EXISTS` statements and keeps no record of applied migrations; the `native-pg` and `worker-cf` contract cells apply it to an empty Postgres on every run, but the schema-parity test under Confirmation does not exist.

## Open Questions

* Q-0042 — which option? (answered round 29, FR-17)
* Q-G3 — **answered (round 5)**: start fresh, no import.
* Q-G1/Q-G2 — upstream version tracking.
* Q-0042.1 — **answered (round 20)**: freeze at the pinned Plane version.
* Q-0042.2 — **moot (round 5/20)**: nothing is imported.

## More Information

[[ADR-0040]], [[ADR-0041]], [[ADR-0043]], [[ADR-0044]], [[ADR-0047]]. `apps/api/plane/db/migrations/`; research `woven/plan:.claude/plans/plane-fork/research/backend-data-model.md` §3.
