---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0001: Hybrid approach — keep the React frontend, rewrite the backend as Workers-native services

## Context and Problem Statement

The long-term goal is an equivalent of Plane that is Cloudflare-native. The repo survey [S] shows a clean seam: the frontend (`apps/web` ~175k LOC; web/admin/space together ~195k LOC, plus ~100k LOC in `packages/*` such as `editor`, `ui`, `propel`, `i18n`) is a separate React Router + MobX + SWR codebase, while the backend is a ~80k LOC Django 4.2/DRF + Celery + RabbitMQ + Postgres + Redis monolith (`apps/api`) with an ~6k LOC Hocuspocus collab server (`apps/live`) that persists only via HTTP calls to Django. Django features such as advisory locks, `ChangeTrackerMixin`, `django-celery-beat`, and channels do not port to Workers. Which parts do we keep and which do we replace?

## Decision Drivers

* Frontend is ~300k LOC of working UI (~195k in the apps plus ~100k in shared packages); rewriting it dominates cost and adds no Cloudflare value.
* Backend is where Cloudflare-nativeness (Workers, Queues, DO, R2, Workflows, Cron) actually applies.
* Must preserve `/api/v1` wire compatibility (ADR-0031) while freeing the core API (ADR-0034; originally ADR-0030, superseded).
* Upstream-sync with Plane's frontend may or may not be required (Q-G2).

## Considered Options

* Option 1: Greenfield rebuild (new frontend + backend, Plane as behavioral reference)
* Option 2: Refactor in place (port Django to Workers/TS incrementally, strangler-fig, keep everything else)
* Option 3: Hybrid — keep React apps and shared packages, rewrite the backend (and collab/proxy layers)
* Option 4: Lift-and-shift (run Django in containers on Cloudflare Containers)

## Decision Outcome

Chosen option: **Option 3, Hybrid**, because it keeps the largest asset (the UI) and concentrates change where Cloudflare primitives apply. Source: interview 2026-10-17, round 1 ("Hybrid: keep frontend, rewrite backend").

### Consequences

* Good, because UI parity is largely inherited; effort focuses on backend behavior and the API contract.
* Good, because the editor/Yjs binary format (`@plane/editor`) is retained, so existing page data stays readable.
* Bad, because the frontend's `@plane/services` and MobX stores assume Django response shapes, so a transitional `compat-plane` adapter must serve them (ADR-0034, which supersedes ADR-0010).
* Bad, because tracking upstream Plane releases will diverge at the service layer (Q-G2).
* Neutral: Option 4 stays as a fallback interim path, not the target.

### Confirmation

A walking-skeleton milestone: the adopted React app, changed only in the four allowed areas (base URL/service layer, auth client, collab provider, branding; ADR-0034), logs in and lists work items against the new backend through the `compat-plane` adapter in both adapter sets.

## Pros and Cons of the Options

### Option 1: Greenfield rebuild

* Good, because no legacy constraints.
* Bad, because ~300k LOC of UI would be re-created for no Cloudflare benefit.

### Option 2: Refactor in place

* Good, because incremental delivery and continuous parity.
* Bad, because Django-shaped internals (ORM, signals, Celery) leak into every step, and the DB is shared during migration.

### Option 3: Hybrid

* Good, because it reuses the UI and rewrites only what must change.
* Bad, because the contract between the two halves has to be managed explicitly.

### Option 4: Lift-and-shift

* Good, because it is fastest to a running system.
* Bad, because it is not Workers-native and does not satisfy the goal.

## Update — interview round 21 (2026-10-18)

Owner direction (round 21): **backend in Rust from the start; Plane's React frontend adopted with minimal changes; a Dioxus (Rust) clone of the UI follows when backend and React frontend are stable, for multi-target builds** (ADR-0013). The hybrid approach remains the **phase-1 shape**; the long-term end state is a Rust UI (ADR-0013). The decision to keep React is therefore transitional, not permanent.

## Update — review fixes, round 25 (2026-10-18)

Confirmation, consequences and links restated for ADR-0034 (no adapted service layer; React changed only in the four allowed areas); LOC figures made consistent (~195k apps + ~100k packages). Substance unchanged.

## Open Questions

* Q-G2 — is upstream sync with Plane a requirement? **Answered (round 4): no, fork and diverge.** The "bad: diverges from upstream" consequence above is therefore accepted, not a risk to mitigate.
* Q-0010 — how the service layer is adapted. **Superseded (round 22): compat-plane adapter, ADR-0034.**

## More Information

Survey: `apps/api`, `apps/live`, `apps/web`, `packages/services`. Related: [[ADR-0002]], [[ADR-0010]] (superseded), [[ADR-0030]] (superseded), [[ADR-0034]], [[ADR-0013]].
