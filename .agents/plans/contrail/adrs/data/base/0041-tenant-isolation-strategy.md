---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0041: Tenant isolation strategy

## Context and Problem Statement

ADR-0003 (accepted) says: single-org first, multi-tenant-capable data layer. We must decide *how* tenants are isolated so that multi-tenancy later is a configuration/ops change rather than a schema rewrite. In Plane the tenant root is `Workspace`; work items, states, cycles, modules are project-scoped (`ProjectBaseModel`: project FK + workspace FK), labels/pages/issue types workspace-scoped (`WorkspaceBaseModel`, nullable project) **[S]**. Some tables are **not** workspace-scoped: `User`, `Profile`, `Account`, `Instance`, `InstanceConfiguration`, `InstanceAdmin` (instance-level) **[S]**. "Single-org" means one instance with potentially several workspaces already (the woven deployment has eleven workspaces per the `plane` skill).

## Decision Drivers

* Depends on ADR-0040 (DB engine) — not every strategy exists on every engine.
* No cross-tenant leakage by construction (defense in depth; see ADR-0141 threat model).
* Cross-workspace features: a user's "my work"/notifications across workspaces, instance admin views (Q-0040.4).
* Operational simplicity: migrations, backup/restore, noisy neighbours.
* Self-hosted twin parity.

## Considered Options

* Option 1: Shared database, row-level tenant key (`workspace_id` on every tenant table) enforced in the repository layer, plus Postgres RLS where available
* Option 2: Database-per-workspace (D1 database or DO-SQLite per workspace)
* Option 3: Schema-per-workspace (Postgres schemas)
* Option 4: Pooled by default, siloed on demand (Option 1 for everyone; promote a heavy tenant to its own database) — a tiered model

## Decision Outcome

Decision pending — see Q-0041. Leading hypothesis: **Option 1 with an Option-4 escape hatch**, confidence **medium**. Every tenant-scoped table carries `workspace_id`; repositories take a mandatory `TenantContext` and cannot issue a query without the predicate (type-enforced); Postgres RLS (`SET LOCAL app.workspace_id`) as defense in depth when the adapter supports it. Instance-level tables live in a separate logical "directory" namespace. This keeps the Plane schema shape (workspace FK already everywhere), eases data import, and leaves room to shard later.

### Consequences

* Good: minimal schema change from Plane; single migration stream; cross-workspace queries stay simple.
* Bad: isolation relies on code + RLS discipline; noisy-neighbour risk; per-tenant backup/delete (GDPR erasure) needs a row-scoped procedure.
* Follow-ups: define which tables are "directory" (global) vs tenant; a lint/test that every new table declares its scope.

### Confirmation

Contract test (ADR-0121): for every repository method, calling with tenant A never returns/mutates tenant-B rows (property test over a two-tenant fixture). CI lint: new tables must declare `scope: tenant|directory`. RLS enabled-check on Postgres adapter.

## Pros and Cons of the Options

### Option 1: Shared DB, row-level key (+RLS)

* Good, because matches Plane's model; simplest ops; cross-tenant admin queries trivial.
* Bad, because a missed predicate leaks data; no physical isolation.

### Option 2: Database-per-workspace

* Good, because physical isolation, easy per-tenant export/delete, aligns with DO-per-workspace collab locality.
* Bad, because migrations must fan out; cross-workspace queries need aggregation; D1 DB-count/management overhead; Postgres analogue is heavy.

### Option 3: Schema-per-workspace

* Good, because logical isolation with one Postgres cluster.
* Bad, because poor with Hyperdrive pooling/`search_path`, migration fan-out, catalog bloat at many tenants; unavailable on SQLite engines.

### Option 4: Pooled + siloed on demand

* Good, because pragmatic growth path.
* Bad, because two operational modes to build and test; router layer needed.

## Update — interview round 11 (2026-10-17)

The owner deferred the choice to the DB spike (Q-0041). Leading hypothesis stays row-level `workspace_id` enforced in the repository layer (R-S1), with Postgres RLS as optional defence-in-depth.

## Update — review fixes, round 25 (2026-10-18)

No substantive change in round 25. The Rust backend enforces the tenant predicate through the `Scope` type the `Authorizer` produces (ADR-0051, round 24): repositories accept only a `Scope`, so a query cannot run without a tenant. Isolation strategy is still deferred to spike 0002.

## Update — review round 29 (2026-10-19)

Per FR-02 (`reviews/2026-10-19-fable-architecture-review.md`), the round-25 sentence above ("repositories accept only a `Scope`, so a query cannot run without a tenant") states the intent, not the code, and round 29 makes it binding. Today the `Database` port takes bare `WorkspaceId` and `ProjectId` arguments, and `insert_activity`, the three outbox methods and `seed` take no tenant at all (`crates/thrust-ports/src/lib.rs:92-138`). `Scope` hands its ids back through public getters (`crates/thrust-core/src/lib.rs:139-155`). The Postgres `Tx::allocate` ignores the workspace (`crates/thrust-adapters-postgres/src/lib.rs:399`), and the membership query joins `project_members` without `m.workspace_id` (`:189-191`). Decision (owner, review §5: apply now):

* Repository and `Tx` methods take `&Scope`, minted only by `authorize`, or a `SystemScope`, minted only by the composition roots and the consumer, instead of bare `WorkspaceId`/`ProjectId`. The adapter derives the tenant key from the scope. This is the "mandatory `TenantContext`" of the leading hypothesis above. The signatures are in ADR-0045 (round 29) and the proof in ADR-0051 (round 29).
* `Scope`'s getters are not public outside `thrust-core`.
* `project_sequences` and the membership join carry `workspace_id`.
* The two-tenant property test in the Confirmation stays, and the "ignore the tenant predicate" mutant fails to compile where the types allow it (ADR-0045, round 29, says which controls can).

The isolation strategy itself (Q-0041) is still open and this ADR stays `proposed`. This note decides only how the repository layer enforces the tenant predicate.

## Open Questions

* Q-0041 — which strategy?
* Q-0040.4 — cross-workspace query needs (drives Option 2 viability). (answered round 29, FR-17)
* Q-G4 — scale targets. (answered round 29, owner)
* Q-0041.1 — Which tables are instance-level "directory" tables (User, Profile, Account, Instance*) and how are they referenced from tenant data (FK across namespaces)? (answered round 29, FR-17)
* Q-0041.2 — Per-tenant data erasure/export requirement (GDPR) and RPO per tenant. (answered round 29, owner default)

## More Information

[[ADR-0003]], [[ADR-0040]], [[ADR-0045]], ADR-0141. Plane scoping: `apps/api/plane/db/models/base.py`.
