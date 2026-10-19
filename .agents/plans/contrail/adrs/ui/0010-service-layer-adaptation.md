---
status: superseded by ADR-0034
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0010: Frontend service-layer adaptation

## Context and Problem Statement

The internal UI API is being redesigned (ADR-0030), but the React apps are kept (ADR-0001). The apps reach the API through `packages/services` (`@plane/services`, ~3.7k LOC) and MobX stores (`apps/web` store folders, `packages/shared-state`) [S]. The stores likely assume Django response shapes, such as paginated envelopes and grouped/sub-grouped issue listings (`group_by`). How do we connect unchanged UI to a new API?

## Decision Drivers

* Minimise edits to ~175k LOC in `apps/web`.
* Typed contract between client and server (ADR-0030).
* Keep the upstream merge surface small if Q-G2 is yes.
* Ability to evolve the API without breaking the UI.

## Considered Options

* Option 1: Regenerate/replace `@plane/services` from the new API contract (new `api-client` package), adapting stores where shapes differ
* Option 2: Compat shim — new backend emits Plane-shaped responses on the internal routes, services unchanged
* Option 3: Adapter layer inside `@plane/services` — services keep their signatures, translate new API to old shapes client-side

## Decision Outcome

Decision pending — see Q-0010. Leading hypothesis: **Option 3 as the migration step, converging to Option 1** (confidence: low-medium) — keep service signatures stable so MobX stores don't change at first, but own the translation in one place so shapes can then be cleaned up incrementally.

> **Update, interview round 4 (2026-10-17):** Q-G2 answered **"No — fork and diverge"**: staying mergeable with upstream Plane is *not* a requirement, so the frontend is owned code. This weakens the main argument for a long-lived compat shim (Option 3). Hypothesis shifts toward **Option 1 (regenerated typed client, stores adapted)**, using Option 3 only as a short-lived stepping stone if the store inventory (Q-0010.1) shows the migration is too large to do in one pass. Confidence: medium.

### Consequences

* Good, because a bounded, testable translation layer limits store churn.
* Bad, because a client-side translation layer is temporary technical debt and costs runtime work.
* Neutral: Option 2 effectively keeps the old internal API (reducing the value of ADR-0030's freedom).

### Confirmation

Inventory (Q-0010.1) of store assumptions; a contract test per service method that compares the shimmed result with golden Plane responses (ADR-0122).

## Pros and Cons of the Options

### Option 1: Regenerate services

* Good, because the cleanest end state and full type safety.
* Bad, because the largest immediate change, with store rewrites.

### Option 2: Server-side compat shim

* Good, because the frontend is untouched.
* Bad, because the legacy shapes are frozen into the new backend.

### Option 3: Client adapter in services

* Good, because incremental, and the server stays clean.
* Bad, because dual maintenance until the shapes converge.

## Update — interview round 17 (2026-10-17)

Accepted. **Regenerate a typed client from the new API** (replacing `@plane/services` over time), not a compat shim (Q-0010). Consequence: stores that assume Django response shapes must be inventoried and adapted (Q-0010.1 stays open as a work item).

## Update — interview round 21 (2026-10-18)

ADR-0010 (regenerate a typed client) was accepted when React was permanent. With the Rust direction, the generated TS client is a **bridge** until ADR-0013 Phase 2; re-confirm the approach against the minimal-change goal (Q-R1, Q-R2).

## Update — interview round 22 (2026-10-18)

**Superseded by ADR-0034**: React keeps using Plane-shaped internal endpoints through a compat adapter, so no regenerated TypeScript client is built. Minimal-change scope (Q-R2): base URL/service layer, auth client, collab provider, branding only.

## Open Questions

* Q-0010 — which approach.
* Q-0010.1 — inventory of store assumptions about Django response shapes.
* Q-G2 — upstream sync requirement.

## More Information

Related: [[ADR-0001]], [[ADR-0030]], [[ADR-0122]]. Paths: `packages/services`, `packages/shared-state`, `apps/web`.
