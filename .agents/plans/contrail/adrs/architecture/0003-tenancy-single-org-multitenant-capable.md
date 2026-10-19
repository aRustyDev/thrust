---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0003: Tenancy — single-org first, multi-tenant-capable data layer

## Context and Problem Statement

The first deployment is a single organization (like today's `projects.woven`). The user does not want to preclude SaaS later. Plane already has a workspace concept [S] (`Workspace`, `WorkspaceMember`, and most domain tables scoped by workspace/project), but also instance-level state (`apps/api/plane/license`, `apps/admin` god-mode). The isolation mechanism (row-level vs DB-per-workspace) depends on the database choice (ADR-0040) and is decided in ADR-0041.

## Decision Drivers

* Ship single-org without paying multi-tenant operational cost.
* Retrofitting a tenant key into every table/query later is expensive and error-prone.
* Per-tenant quotas and isolation may become a requirement.
* Must work across adapter sets (ADR-0002).

## Considered Options

* Option 1: Single-org only; no tenant concept beyond Plane's workspaces
* Option 2: Full multi-tenant SaaS from day one (billing, quotas, per-tenant stores)
* Option 3: Single-org first, with a tenant key and tenant-scoped repositories built in from the start

## Decision Outcome

Chosen option: **Option 3**, because it keeps v1 simple while keeping the SaaS door open. Source: interview 2026-10-17, round 1 ("Single-org first, multi-tenant-capable").

Commitments: every aggregate carries a tenant key; repositories require a tenant context and refuse unscoped queries; instance-level (cross-tenant) operations are an explicit, separate privileged path. The isolation mechanism is deferred to ADR-0041.

### Consequences

* Good, because avoids a painful retrofit.
* Bad, because a small cost on every query/repository from day one.
* Neutral: billing, quotas, and tenant provisioning are explicitly out of scope until needed.

### Confirmation

A repository-level test asserts that no repository method runs without a tenant context; a negative cross-tenant read test belongs in the contract suite (ADR-0121).

## Pros and Cons of the Options

### Option 1: Single-org only

* Good, because simplest.
* Bad, because a SaaS pivot would mean a rewrite of the data access layer.

### Option 2: SaaS day one

* Good, because it exercises isolation early.
* Bad, because it builds features nobody has asked for.

### Option 3: Single-org first, multi-tenant-capable

* Good, because proportional cost.
* Bad, because the capability is unproven until a second tenant exists.

## Open Questions

* Q-0041 — tenant isolation mechanism.
* Q-0040.4 — whether cross-workspace queries are needed.

## More Information

Related: [[ADR-0040]], [[ADR-0041]], [[ADR-0141]].
