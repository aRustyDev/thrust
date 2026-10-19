# Spike 0009 — Compat-plane inventory and sizing

**Gates:** ADR-0034, F-X13 (P0), PLAN.md scoping.
**Questions:** Q-0010.1, Q-0011.1.
**Timebox:** 3 working days. **Depends on:** spike 0008 (pinned instance + recording harness). **Owner:** TBD. Run **before** P0 planning.

## Question
Which of Plane's internal endpoints and response shapes does the adopted React app actually use, how big is the
`compat-plane` facade, and what is the minimum slice for P0?

## Facts to start from (review, round 25)
`plane.app` exposes ≈233 route entries, plus ≈66 auth/public/space entries and ≈70 `/api/v1` entries. drf-spectacular
covers `/api/v1` only, so **the internal API has no schema**; the inventory must come from React store/service code and
recorded traffic. The flow includes grouped/sub-grouped issue listings, pagination payloads, Django CSRF/session cookie
behaviour and server-redirect `/auth/*` form posts.

## Method
1. Static inventory: every call site in `apps/web`, `apps/space`, `packages/services` → method, path, params, response type used.
2. Dynamic inventory: run the React app against the pinned oracle with a scripted P0 user journey set; record all requests
   (HAR) with response bodies; diff static vs dynamic.
3. Rank endpoints by call frequency and by P0 feature (FEATURES.md); mark each endpoint P0 / P1 / drop / not used.
4. Shape catalogue: group_by / sub_group_by payloads, pagination envelope, expand params, error bodies, cookie/CSRF flow.
5. Effort estimate per endpoint class (CRUD passthrough, grouped listing, bulk, file upload, auth redirect) and the
   minimum slice that lets one user create a workspace, project and work item through the unmodified React UI.
6. Identify React behaviours that depend on endpoint quirks and would be cheaper to fix in the allowed change areas
   (base URL/service layer, auth client, collab provider, branding — Q-R2).

## Exit criteria
Committed inventory table (endpoint, frequency, tier, shape notes), size estimate in person-days with a P0 minimum slice,
and a recorded-journey replay harness the compat adapter must pass. If the P0 slice exceeds an agreed cap, reopen
ADR-0034 (option: adapt the React service layer instead).

## Deliverables
`research/0003-compat-plane-inventory.md`; Q-0010.1 closed; F-X13 acceptance list.

## Result (round 26)

**Desk half done**, on the pre-pin and the pinned trees. Findings: `research/0003-compat-plane-inventory.md`; ADR-0034 *Update* has the sizing. The HAR recording, call frequency and the replay harness are the dynamic half, next.
