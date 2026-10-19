---
status: superseded by ADR-0034
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0030: Internal API style (and real-time push)

## Context and Problem Statement

The internal UI API (Plane's `api/` → `plane.app`, with views grouped by domain: analytic, asset, cycle, estimate, issue, module, notification, page, project, search, state, user, view, webhook, workspace [S]) is being redesigned; only `/api/v1` is kept compatible (ADR-0031). The style must suit a TypeScript client (`@plane/services` replacement, ADR-0010) and Workers. Separately, Plane clients today rely on SWR fetching; real-time push of work-item changes is not first-class (F-X09).

## Decision Drivers

* End-to-end types shared with the TS frontend.
* Coexistence with the REST+OpenAPI public v1 on the same app.
* Portability across adapter sets (the HTTP layer must run on Workers and Node).
* Ability to serve non-TS clients/tooling later.

## Considered Options

* Option 1: Hono + typed RPC (Zod schemas, generated typed client); public v1 as plain REST + OpenAPI in the same app
* Option 2: tRPC for internal; v1 REST separate
* Option 3: REST + OpenAPI everywhere; client generated from the spec
* Option 4 (realtime sub-decision): none / SWR polling vs SSE vs WebSocket via Durable Object per workspace

## Decision Outcome

Decision pending — see Q-0030. Leading hypothesis: **Option 1** (confidence: medium) because it shares types with the client, runs on Workers and Node, and sits beside OpenAPI-described v1 routes. Real-time push: **start with polling/SWR (as today), design the event contract (ADR-0046) so SSE/WebSocket push can be added** (confidence: medium; Q-0030.1).

### Consequences

* Good, because typed client with low boilerplate.
* Bad, because RPC-style clients are less discoverable to non-TS consumers; two API styles in one app.
* Neutral: the real-time push option introduces a per-workspace DO fan-out; defer unless needed.

### Confirmation

Spike: implement `GET/POST work-items` in the chosen style with the generated client used by one `apps/web` view; measure bundle impact and verify v1 coexists.

## Pros and Cons of the Options

### Option 1: Hono typed RPC

* Good, because lightweight; shared Zod; fits Workers and Node.
* Bad, because ties internal API to TS clients; a smaller community than REST.

### Option 2: tRPC

* Good, because mature end-to-end typing.
* Bad, because heavier adapters on Workers; separate v1 REST stack.

### Option 3: REST + OpenAPI

* Good, because one style, language-neutral, contract-first with codegen.
* Bad, because more boilerplate; client generation quality varies.

### Option 4: Realtime

* Good (SSE/WS), because instant UI updates.
* Bad, because DO fan-out cost and complexity; polling is simplest.

## Update — interview round 9 (2026-10-17)

The owner chose the recommended direction: **Hono + typed RPC, with no live push at P0** (clients refetch/poll; live push arrives with collaboration in P1) (Q-0030, Q-0030.1). The ADR stays `proposed` until the frontend store inventory (Q-0010.1) confirms the typed client can replace `@plane/services` without reshaping grouped listings/pagination; Q-0030.2 (shared vs separate app with `/api/v1`) remains open.

## Update — interview round 21 (2026-10-18)

The round-9 leaning (Hono + typed RPC) is **superseded by the Rust decision**. Candidate styles now: REST + OpenAPI generated from Rust (`axum` + `utoipa`-style) with a generated TS client for React and a generated or shared-crate Rust client for Dioxus; or Dioxus server functions for the Rust UI only. Because the React frontend is transitional, whether to build a *new* internal API at all, or serve Plane-shaped internal endpoints to React, is open (Q-R1, Q-R5).

## Update — interview round 22 (2026-10-18)

**Superseded by ADR-0034**: style = REST + OpenAPI core API plus a transitional Plane-compat adapter. Code-first vs spec-first remains in ADR-0032.

## Open Questions

* Q-0030 — API style.
* Q-0030.1 — real-time push mechanism.
* Q-0032 — OpenAPI as source of truth.

## More Information

Related: [[ADR-0006]], [[ADR-0010]], [[ADR-0031]], [[ADR-0032]], [[ADR-0046]]. Paths: `apps/api/plane/app/views`, `packages/services`.
