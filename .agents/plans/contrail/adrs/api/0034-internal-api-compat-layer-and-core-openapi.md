---
status: accepted
date: 2026-10-18
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0034: Internal API strategy — Plane-compat layer for React, one OpenAPI core API for Dioxus and v1

## Context and Problem Statement

Round 2 chose "compatible v1 only, new internal API" and ADR-0010 chose a regenerated typed TS client. Round 21 made the React
frontend **transitional** (adopted with minimal changes, later replaced by a Dioxus clone, ADR-0013). Investing in a
redesigned internal API plus a generated TS client for a UI that will be replaced is partly throwaway.

## Decision Drivers

* Minimal changes to the React app (Q-R2).
* One clean, typed API for the long-term Rust UI and for public `/api/v1`.
* Do not freeze Django-era response shapes into the Rust design.
* Delete the transitional layer cleanly when Dioxus lands.

## Considered Options

* Option 1: Plane-shaped internal endpoints (compat layer) for React **plus** one new OpenAPI core API that Dioxus and `/api/v1` build on
* Option 2: New internal API + generated TS client for React (previous direction, ADR-0010/0030)
* Option 3: Compat only; Dioxus uses the same Plane-shaped API

## Decision Outcome

Chosen: **Option 1** (owner, round 22). Architecture: use-cases in `core` are the single implementation; **three thin driving surfaces** sit on top —
(1) a **`compat-plane` HTTP adapter** that reproduces the internal endpoints/response shapes the React app expects
(grouped issues, pagination, `group_by` payloads); (2) **`/api/v1`**, Plane-shaped and wire-compatible (ADR-0031), a thin shim over the same core use-cases; and
(3) the **core REST+OpenAPI API** (axum, code-first per ADR-0032) used by the Dioxus clients, scripts and MCP. The compat adapter is **deleted** when
ADR-0013 Phase 2 completes. Real-time push stays out of P0 (round 9).

### Consequences

* Good: React runs with near-zero change; the core API is designed for Rust and multi-target clients; `/api/v1` shares the core use-cases.
* Bad: three HTTP surfaces during the transition (compat-plane, `/api/v1`, core API); the compat layer must be derived from Plane's actual internal endpoints
  (inventory needed, Q-0010.1) and kept behaviourally faithful (oracle tests, ADR-0122).
* Neutral: supersedes ADR-0010 (regenerated TS client) and ADR-0030 (Hono/typed-RPC leaning).

### Confirmation

Inventory of the internal endpoints the React stores call; the compat adapter passes the pinned-oracle response diff for
those endpoints; the compat crate has no imports from adapters (it is a driving adapter only) so it can be removed.

## Update — review fixes, round 25 (2026-10-18)

Clarified that there are **three** surfaces (compat-plane, `/api/v1`, core API) and that `/api/v1` is a thin Plane-shaped shim over core use-cases; marked the OpenAPI source-of-truth question answered by ADR-0032.

## Update — spike results, round 26 (2026-10-18)

**Spike 0009, desk half** (`research/0003-compat-plane-inventory.md`; scripts in `scripts/compat-inventory/`, re-run with `--root <tree>`). It ran on the pre-pin tree (`300c2009d4`) and on the pinned oracle tree (`ba85d4d3d3`); the pinned numbers are quoted here, and the pin moved the slice estimate by 0.7 person-days (0.3 % of P0, inside the model's own band). Everything is from source: there is no recorded traffic, call frequency or response body yet — that is the dynamic half, which now has its harness from spike 0008. Counts are computed; person-day figures are the spike's judgement (unit costs per row class and handler size, plus 30 pd of shared machinery).

* **Surface (pinned):** internal `/api/` 233 entries (441 method-by-path rows); auth 41 and space 25; instance/license 14; `/api/v1` 73 (the pin added five `-lite` GET routes). The compat surface is **545 rows**, of which the React tree never calls **205** (39 % of the internal rows, including all 13 PUT rows and every `intake-issues`, `intakes` and `inboxes` alias). Tiers over the 545: **P0 290**, P1 29, P2 8, admin-only 13, not used 205. Twenty-two call sites look live but hit routes this server does not have (EE dashboards, integrations, Slack/GitHub, `bulk-operation-issues`, `PATCH sidebar-preferences/{key}/`); the oracle answers them with 404, so the adapter must too.
* **Sizing (data for Q-0009.1):** the **minimum P0 slice** — sign up, onboard, create a workspace, a project and a work item, and see it in the list through the unmodified UI — is **48 routes on 42 paths (+3 to open the item), 39.3 person-days likely (26.7–66.7)**, of which 12.5 is shared machinery (CSRF/session/redirect, pagination, grouping). **All P0 rows the React tree uses: 290 rows, 207.6 pd (139.1–364.2), 5.3 times the slice.** Inside that, about 16 pd is the `/auth/*` 302 emulation, about 21 the presigned asset flow, and about 43 the group-by, filter, pagination and expand machinery. Whether that breaches the cap, and whether to reopen this ADR (adapt the React service layer), is the owner's call.
* **The biggest lever is inside the allowed change areas (Q-R2): the auth client** (13 files, 21 reference sites). Replacing the CSRF-token + HTML-form-POST + `302 ?error_code=` protocol with JSON POSTs to the identity adapter that return the same `error_code` values (so `authErrorHandler` is unchanged) removes about 16 pd of emulation and 41 of the 545 rows. The service layer can also point comment reads at a native list instead of `history/?activity_type=issue-comment` and make the project-cover upload a single POST (the 19 slashless calls are moot at the pin: the pinned client appends `/` to every API URL, catalogue PQ-109 and PQ-514); the default project cover itself is a component edit outside the Q-R2 list.
* **Quirks the adapter must reproduce** (results file, section 10): auth is a browser-form + 302 protocol with 60 error codes and an HttpOnly CSRF cookie; the unauthenticated `GET /api/users/me/` must be a JSON object without `id`; `GET /api/instances/` must carry `is_setup_done: true`; **401 must mean exactly "no or expired session"** (any other 401 makes the SPA hard-redirect to sign-in; role failures are 403); the issue listing contract (12-key envelope, cursor `per_page:page:is_prev`, three result shapes, a `"None"` group, default `per_page` 1000, rich `filters` JSON plus 25 legacy params); creating a project drags in the presigned asset flow and a browser-reachable object store; every workspace page fires 9 reads and every project page 13, including P1-tier ones (`project-stats`, `intake-state`); `POST /api/workspaces/` dispatches `workspace_seed`, which creates a bot user and a sample project.
* **The pin changed behaviour, not shape:** 123 of the 545 compat rows (77 of the P0 rows) have changed handler bodies, mostly upstream security hardening (tenant scoping, role tightening, `order_by` and `group_by` allowlists, auth throttling at 10 per minute per IP — inferred from source). The model prices size, not the number of authorisation branches, so the extra 403, 404 and silent-exclusion cases are priced by the replay harness, not by the estimate. About 84 changed handlers were not read.
* **F-X13 acceptance list (proposed):** the 290 rows tagged P0 in `scripts/compat-inventory/out-pinned/endpoint-inventory.csv`, slice first, each passing the recorded-journey replay.
* **Not done:** the HAR recording and call frequency, response bodies and empty-state shapes, error bodies and cookies, the 22 live-looking 404s, workspace seeding, and the woven OIDC routes on the running pin (the four `/auth/oidc/…` routes are byte-identical to the pre-pin tree). Q-0010.1 closes when these are confirmed.

## Update — owner decision, round 27 (2026-10-18)

Owner decision on Q-0009.1: **tripwires, not a budget.** Build the 48-route minimum slice first (estimate 39.3 person-days) and re-estimate from the actual cost per endpoint class. **Reopen this ADR** (adapt the React service layer instead of emulating Plane's internal API) if the slice actually costs more than **66 pd** or the re-estimated P0 total exceeds **364 pd** (the model's high bounds, `research/0003` section 15.6). Before reopening, apply the cheap levers that are inside the allowed change areas: the auth-client JSON change (about 16 pd, 41 of the 545 rows), a native comments list and a single-POST cover upload (trailing-slash normalisation is moot at the pin).

## Update — review round 29 (2026-10-19)

Per FR-01 and FR-06 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply
now, with the M2 screen list confirmed as the review's default and two surfaces at P0), the acceptance list and the
P0 surfaces change as follows.

**Acceptance list (FR-01).** The F-X13 acceptance list is the 48-route minimum slice (`research/0003` §8.1 and §15.6;
issue #44, M1) plus the compat rows behind the M2 daily-use screens (`PLAN.md`, Q-R8; issue #46, M2): work-item list
(group-by, filters, pagination); work-item detail (description, comments, activity, attachments, labels, assignees,
state, priority, dates); project create and settings; workspace members (no invites); in-app notifications; global
search. This replaces the round-26 proposal of the 290 P0 rows. Every other route the React shell calls answers with
the cheapest shape-correct stub (an empty list, a 404 or the recorded fixture), and each stub is recorded under
deviation D-021. The Playwright journey suite over the M2 screens (issue #45) is the UI parity matrix (TRACEABILITY)
and the UI oracle for the Dioxus port. Compat rows are deleted as each screen moves to Dioxus (ADR-0013, round 29), so the
adapter shrinks screen by screen and its last row goes when Phase 2 completes. The round-27 tripwires apply to the
acceptance list, not to the 290 rows (Q-0009.1).

**Two surfaces at P0 (FR-06).** At P0 the use-cases carry two driving surfaces: compat-plane and `/api/v1`. The core
REST+OpenAPI surface (item 3 of the Decision) has no P0 consumer: the Dioxus client is M4, MCP is P1, and the P0 AI
access is the public `/api/v1` (F-X08). It is generated from the `thrust-core` use-cases at M4, when the first Dioxus
client exists (ADR-0032, round 29). This supersedes the Decision's "three thin driving surfaces" and the round-25
"three surfaces" clarification until M4; the Consequences' "three HTTP surfaces during the transition" reads two until
then. The slice's own routes (`crates/thrust-api/src/lib.rs:269-293`) are mounted under `/api/v1/` but are neither
Plane-shaped nor documented; they move out of that prefix, which is reserved for wire-compatible routes (issue #47,
M0). Keeping Plane-shaped request and response types out of the shared crates is FR-20, tracked as issue #52.

## Open Questions

* Q-R5 / Q-0032 — answered (round 24): REST + OpenAPI, code-first, hybrid for v1 (ADR-0032).
* Q-0010.1 — which internal endpoints/response shapes React depends on (the compat inventory). (answered round 29, FR-01, FR-17)

## More Information

Supersedes [[ADR-0010]] and [[ADR-0030]]. Related: [[ADR-0013]], [[ADR-0031]], [[ADR-0032]], [[ADR-0122]].
