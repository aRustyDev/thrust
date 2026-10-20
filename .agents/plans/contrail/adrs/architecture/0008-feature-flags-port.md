---
status: accepted
date: 2026-10-18
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0008: Feature flags port

## Context and Problem Statement

Feature flags are one of the four P0 foundations (F-E00a, round 4). Open-EE gated features behind flags resolved per
instance/workspace **[O]**. Plane CE has instance-level config but no flag system. Flags gate capabilities in the
`Authorizer` (ADR-0051), UI affordances and rollout of Open-EE-equivalent features (custom roles, custom fields, …).

## Decision Drivers

* Per-instance and per-workspace evaluation, cheap in the request path, no external dependency at P0.
* Same behaviour on both adapter sets; testable.
* Admin-editable without redeploy (workspace settings, ADR-0021).

## Considered Options

* Option 1: A `Flags` port backed by a database table (`key`, `scope` instance|workspace, `enabled`, optional rules), read per request and memoised for the request
* Option 2: Static configuration (env/TOML) only, change = redeploy
* Option 3: OpenFeature-style port with an external provider (Unleash/flagd) adapter

## Decision Outcome

**Decision (accepted, round 29): Option 1 without the port: a `flags` table (`key`, `scope` instance|workspace, `enabled`, optional rules) read per request and memoised for the request; there is no `Flags` port. Per FR-18, `reviews/2026-10-19-fable-architecture-review.md` §5.**

Earlier leading hypothesis, kept as rationale: **Option 1** (confidence: medium): consistent with "no cache at P0", editable at runtime, trivial self-hosted; keep the port OpenFeature-compatible in semantics so Option 3 can be added as an adapter. The port and its OpenFeature-compatible seam are dropped (round-29 note).

### Consequences

* Good: runtime control, audit via ADR-0048, no new infra.
* Bad: one more indexed read per request unless memoised; flag hygiene (removal) is on us.

### Confirmation

Contract tests for flag evaluation (instance default, workspace override, unknown key = off); a lint that new capabilities declare their flag.

## Update — review round 29 (2026-10-19)

Per FR-18 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: backlog, plus two tier edits, "F-E00a flags as a table with no port"), this ADR is accepted as Option 1 without the port, and Q-0008 is answered. FR-17 lists it among the ADRs to accept on their leading hypotheses (and Q-0008 among the questions whose default is already in their own text), and FR-18 asks for "no ADR beyond a short accepted 0008". FEATURES F-E00a reads "a `flags` table read per request; no port" (round 29).

Flags are an ordinary table in the system-of-record database, read per request like any other table. No `Flags` port, no per-set adapter and no OpenFeature-compatible seam are built. FR-18 places this among the foundations that can be cheaper at P0. No `flags` table exists in the slice yet (the migrations in `crates/thrust-adapters-postgres/migrations/` define none). Admins edit flags without a redeploy, through the workspace settings (ADR-0021), as the third Decision Driver requires.

This supersedes the leading hypothesis's "keep the port OpenFeature-compatible in semantics so Option 3 can be added as an adapter" and the port in this ADR's title. The Confirmation's cases (instance default, workspace override, unknown key = off) stay; with no port they test the evaluation, not a port contract. The Consequences' "audit via ADR-0048" applies from P1, when ADR-0048's audit rows arrive (ADR-0048, round 29).

## Open Questions

* Q-0008 — flag storage/evaluation model and who may edit flags. (answered round 29, FR-18)

## More Information

Related: [[ADR-0021]], [[ADR-0051]], [[ADR-0048]]. Features: F-E00a.
