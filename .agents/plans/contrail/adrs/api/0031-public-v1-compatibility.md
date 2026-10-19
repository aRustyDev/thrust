---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0031: Public `/api/v1` is kept wire-compatible with Plane

## Context and Problem Statement

Plane exposes a public REST API at `api/v1/` (`plane.api`) authenticated by `X-Api-Key` (`APIToken`), covering assets, cycles, intake, invites, labels, members, modules, projects, states, stickies, users, and work items [S]. External scripts and integrations may depend on it. The internal UI API will be redesigned (ADR-0034); the question is how strictly to treat v1.

## Decision Drivers

* External clients (scripts, integrations, Woven tooling) depend on v1.
* The `drf-spectacular` schema (`api/schema/`) gives a machine-readable contract oracle.
* The internal API redesign must not drag v1 with it.

## Considered Options

* Option 1: Keep `/api/v1` wire-compatible (paths, shapes, auth header, status codes); use the OpenAPI schema as the contract-test oracle
* Option 2: Redesign everything, provide a migration guide
* Option 3: Compatible subset only (the most-used resources), rest dropped

## Decision Outcome

Chosen option: **Option 1**, because v1 is the stable external contract, and compatibility is cheap to verify with an oracle. Source: interview 2026-10-17, round 3 ("Compatible v1 only, new internal API").

### Consequences

* Good, because existing clients and tokens keep working (token migration: Q-0052).
* Bad, because v1 quirks (pagination, field names) are carried into the new backend.
* Neutral: tiering per resource remains open in FEATURES.md section A5.

### Confirmation

A contract-test suite generated from the Plane OpenAPI schema runs against the new backend, plus a response-diff harness against a live Plane (ADR-0122).

## Pros and Cons of the Options

### Option 1: Wire-compatible

* Good, because zero client breakage.
* Bad, because inherits legacy quirks.

### Option 2: Full redesign

* Good, because the cleanest API.
* Bad, because breaks clients.

### Option 3: Subset

* Good, because less work.
* Bad, because unknown clients may use dropped resources.

## Update — review fixes, round 25 (2026-10-18)

References to ADR-0030 (superseded) now cite **ADR-0034** (internal API strategy: `compat-plane` adapter, `/api/v1` and the core API). `/api/v1` remains Plane-shaped and wire-compatible; it is implemented as a thin surface over the same core use-cases.

## Update — spike results, round 26 (2026-10-18)

Spike 0008 measured the surface this ADR promises to match: the pinned instance routes **135 v1 operations, 130 of them in its schema**. **Estimates are not routed** (404 on Plane too), so they were removed from the resource list above and need no matching. The corpus records Plane behaviours that the Rust implementation must either reproduce or fix (Q-0033.2, Q-0122.3); the register recommends `reproduce` for ten entries and `fix` for the 500s (EX-001 and EX-002, one failure seen through two checks):

* only a **missing** API key returns 401; an invalid, expired or inactive key returns **403** `{"detail":"Given API token is not valid"}`;
* two error-body shapes coexist: `{"detail": …}` (DRF) and `{"error": …}` (Plane);
* `PATCH` or `DELETE` on an unknown project returns 403, not 404; state create returns 200, not 201; a project can be created with `name=123`; member delete is idempotent (204);
* `per_page` above the maximum returns 400, but 0 or −1 return 500; `POST /workspaces/{slug}/assets/` always returns 500 (`api/views/asset.py` passes `is_server=True`, which the storage constructor does not accept; **upstream has the same bug**);
* `GET /api/v1/users/me/` has an undeclared-null `avatar_url`; the fork's upload responses carry an extra `upload_data.method` that the schema does not describe.

## Update — owner decisions and spike results, round 27 (2026-10-18)

Owner decision, round 27 (Q-0033.2): **reproduce every Plane quirk the oracle records, including all of Plane's 500 responses; no `fix` deviation is approved**, and every Plane quirk is to be documented for planning and context (`research/0007-plane-quirks-catalog.md`, bead `plane-byd.5`). So the register entries EX-001 and EX-002 (unhandled exceptions returning 500) change from `rust_expectation: fix` to `reproduce`; the register file is edited the next time the oracle branch is touched. Reading to confirm: "reproduce" means returning the same status code (and, where observable, the same body shape) as a **deliberate, handled error**, never a crash of the process (ADR-0006 panic policy).

## Update — quirks catalogue findings, round 27 (2026-10-18)

The quirks catalogue (`research/0007`) lists 157 Plane behaviours for planning; the default for every entry is **reproduce** under this ADR, except that the owner has not yet decided the two security-relevant defects PQ-412 (any valid API key reads any project's attachment metadata) and PQ-413 (`cycle-issues/` re-points other projects' or tenants' issues). Those conflict with the tenant predicate in SPEC R-S1 and ADR-0141 (Q-0033.3). Recommendation: fix them as recorded deviations with an oracle exception, because scope enforcement is not negotiable.

## Update — owner decision, round 27 (2026-10-18)

Owner decision on Q-0033.3: **fix** the two security-relevant Plane defects as recorded deviations — **D-014** (PQ-412: `issue-attachments/` and `attachments/` list routes enforce project membership) and **D-015** (PQ-413: `cycle-issues/` scopes the lookup of existing `CycleIssue` rows to the workspace and project) — each with an oracle exception so the diff stays clean. Everything else in the quirks catalogue keeps the default `reproduce`.

## Open Questions

* Q-G8 — are there clients beyond the web apps that make strictness more important?
* Q-0122 — design of the v1 oracle.
* Q-0052 — compatibility of existing API-key values at migration.

## More Information

Related: [[ADR-0034]], [[ADR-0032]], [[ADR-0033]], [[ADR-0122]]. Paths: `apps/api/plane/api`.
