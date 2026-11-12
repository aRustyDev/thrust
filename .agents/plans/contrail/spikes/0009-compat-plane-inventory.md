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

## Dynamic half, first part (issue #45, 2026-11-12)

The stock app, built from the pin, run against the pinned oracle in a real browser by the Playwright journey of M1 (`e2e/journeys/m1-sign-up-to-work-item.spec.ts`; `scripts/e2e.sh plane-oracle`, which says how: `tools/e2e-plane/README.md`), recorded as a HAR **with the bodies of the `/api/` and `/auth/` calls**:

- **`e2e/har/m1-sign-up-to-work-item.plane.har`**: 90 entries (sign-up, onboarding, a workspace, a project and its cover, a work item and the list that shows it), 366 KB, from the pin `oracle/v1-pin-20261018`. `tools/e2e-har/scrub.py` took out the cookie, CSRF and password values, the presigned-URL signature and the volatile times; the URLs, statuses, other headers and bodies are as recorded, with the origin the run used (`http://localhost:18088`) and the ids not renumbered. What the check on it found and did not: G-31 in `docs/src/testing/status.md`.
- It is the **M1 journey's** only. Each M2 screen's HAR is committed with its journey (#46), which is also how its acceptance list is derived (#46's proposal). The call frequency and the static-versus-dynamic diff (method step 2) are not done; **the replay harness that reads these is still to build**.

To record again: `E2E_RECORD_HAR=1 ORACLE_DIR=<fork checkout>/oracle scripts/e2e.sh plane-oracle`. It takes the oracle stack (a fresh one, then down), so it drops the seeded snapshot that `scripts/oracle-v1.sh plane` needs.
