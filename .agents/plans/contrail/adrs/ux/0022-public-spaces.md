---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0022: Public spaces (published boards)

## Context and Problem Statement

`apps/space` (served at `/spaces`) renders published project boards from `plane.space` (`api/public/`) with anonymous access, backed by `ProjectDeployBoard` and related public-member models [S]. The Woven deployment sits behind Cloudflare Access/WARP, where "anonymous" is itself problematic (Access gates all hosts). Public spaces also widen the attack surface (data exposure, scraping, abuse).

## Decision Drivers

* Whether any user needs external/anonymous publishing at all.
* Access-gating vs public routes (ADR-0050).
* Data-minimisation and abuse controls (ADR-0141).
* Cost of keeping a whole app and API surface (`api/public`).

## Considered Options

* Option 1: Include in scope; anonymous `api/public` equivalent, bypassing Access on a dedicated hostname/path
* Option 2: Include, but only for authenticated viewers (Access-gated "internal spaces")
* Option 3: Defer or drop public spaces (tier P2/drop)

## Decision Outcome

**Decision (accepted, rounds 8 and 19): Option 1 — unauthenticated public spaces with explicit publish per project, P0 (F-D09), served by a separate cached Worker on its own public hostname.** (Earlier hypothesis, superseded: Option 3 defer with Option 2 as the first increment.)

### Consequences

* Good, because published boards reach parity and need no Access login for readers.
* Bad, because it adds an unauthenticated attack surface: only explicitly published fields are served, 60 req/min/IP with caching, and the threat model is ADR-0141.
* Neutral: tenancy rules must cover anonymous reads; the Access bypass is scoped by hostname, not path.

### Confirmation

Decision recorded in FEATURES.md (F-D09, P0) and a threat-model section in ADR-0141 covering anonymous reads and rate limiting.

## Pros and Cons of the Options

### Option 1: Full anonymous

* Good, because full parity.
* Bad, because it needs an Access bypass, rate limiting, and data-exposure review.

### Option 2: Authenticated only

* Good, because it keeps the Access posture.
* Bad, because not "public", and still needs a space-specific read model.

### Option 3: Defer/drop

* Good, because the smallest scope and risk.
* Bad, because loses the feature.

## Update — interview round 8 (2026-10-17)

Owner direction: public spaces are **P0 (F-D09)** and **unauthenticated public with explicit publish per project** (Q-0022, Q-0141.2 answered). Implications: a dedicated hostname/route that bypasses Cloudflare Access; read-only API surface separate from the authenticated one; only fields explicitly published are serialized (data minimisation, ADR-0141); per-IP/per-board rate limits for unauthenticated traffic (Q-0141.3, ADR-0033); public asset serving path (Q-0047.2). The remaining details (hosting of the `space` app, caching, abuse controls) were chosen in round 19 (below).

## Update — interview round 19 (2026-10-18)

Accepted. **The `space` app is served on its own public hostname by a separate Worker with caching**, keeping the Access bypass narrow; unauthenticated, explicit publish per project; 60 req/min/IP (round 17); assets via Worker proxy (round 16).

## Update — review fixes, round 25 (2026-10-18)

Decision line, consequences and Confirmation rewritten to the accepted unauthenticated-P0 outcome; the round-8 'stays proposed' sentence is marked resolved. Substance unchanged.

## Update — spike results, round 26 (2026-10-18)

Spike 0009 finding: the `space` app is **server-rendered** (`ssr: true` in `apps/space/react-router.config.ts`, in both trees; its loader fetches the API), so "a separate cached Worker" cannot simply serve static assets (ADR-0011 is corrected). The separate Worker has to run the React Router SSR build, or `space` has to be built as an SPA (the page body is client-rendered either way; only the per-board link-preview `<head>` is server-rendered, see the ADR-0011 round-27 feasibility note; it edits a loader outside the Q-R2 change list). Either way the Worker has no gated routes of its own: in spike 0004 run 1 the stand-in `space` Worker answered on its own hostname, and `/internal/worker-secret`, `/gated/asset-secret.txt` and `/internal/ws` on it were all 404 `[tested-run1]` — which says nothing yet about how narrow the Access bypass is. Decision: Q-0011.2 (owner).

## Update — owner decisions and spike results, round 27 (2026-10-18)

Spike 0004 run 2 (real platform): the public `space` hostname answered without credentials and served none of the product host's gated content (`/internal/worker-secret`, `/gated/asset-secret.txt` and `/internal/ws` were 404); twelve SNI/Host-confusion probes between the two hostnames exposed nothing, and a real mismatch answers 403. This is evidence for the narrow bypass, with a weaker test than the traversal one (only 3 of the 12 probes resolved to the gated target). The SSR question (Q-0011.2) is still open.

## Update — owner decision, round 27 (2026-10-18)

Owner decision (Q-0011.2, round 27): the separate public Worker for `space` runs the React Router SSR build (ADR-0011). The feasibility build under workerd comes first.

## Update — space SSR feasibility result, round 27 (2026-10-18)

From the `space` SSR feasibility build (ADR-0011): (1) the published-board URLs that the API generates must match the `space` hostname and base path (the SSR Worker builds at the root path, not `/spaces`); (2) the browser calls the API itself, so the public API needs CORS with credentials from the `space` hostname or a same-origin allow-list proxy (Q-0011.4); (3) cache the SSR response with a short explicit TTL, keyed on the anchor, and purge on publish and unpublish, because unknown boards return 200.

## Update — space hosting decision (Q-0011.3, supersedes the SSR Worker), round 27 (2026-10-18)

Owner decision on Q-0011.3: the public `space` hostname serves the **SPA plus a Rust handler that injects the per-board link-preview metadata** (ADR-0011); the earlier "SSR Worker" decision is superseded. Escape every injected value; cache with a short TTL keyed on the anchor and purge on publish and unpublish.

## Update — review round 29 (2026-10-19)

Per FR-11 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now, "ADR-0090 accepted as Option 1 for P0 with a reopen trigger"), the `space` hostname is a route group of the one Worker (ADR-0090, round 29), not a separate Worker. That Worker serves the `space` SPA, the head-injection handler and the public API on the `space` hostname, so the browser calls the API same-origin and needs no CORS; this answers point (2) of the round-27 feasibility note (Q-0011.4, answered round 29 with FR-11's default; review §4, "`space` as an SPA with Rust head injection").

This supersedes only "served by a separate cached Worker" in the Decision Outcome and "by a separate Worker with caching" in the round-19 note. Unchanged: public spaces keep their own public hostname, so the Access bypass stays scoped by hostname (Consequences); the response is cached with a short TTL keyed on the anchor and purged on publish and unpublish (round 27); the limit is 60 req/min/IP (round 17); and publishing is explicit per project.

## Open Questions

* Q-0022 — scope and tier of public spaces.
* Q-0141 — threat model for public spaces. (answered round 29, FR-17)

## More Information

Related: [[ADR-0050]], [[ADR-0141]]. Paths: `apps/space`, `apps/api/plane/space`.
