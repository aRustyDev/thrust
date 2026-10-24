---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0141: Threat model for tenancy, API keys, public spaces, webhooks and collab

## Context and Problem Statement

The architecture introduces trust boundaries that Plane's Django monolith handled implicitly: Cloudflare Access / WARP
as outer gate (ADR-0050), API keys (`X-Api-Key`), a multi-tenant-capable data layer (ADR-0003/0041), anonymous
published boards (`apps/space`, `api/public`), outbound webhooks and link unfurling (Plane already has a URL-security
test in `apps/api` **[S]**), file uploads, collab WebSockets, and asynchronous queue messages. Tenant isolation bugs and
SSRF are the highest-impact classes. The status probe is a `non_identity` WARP session — Access-gating a probed host
breaks it **[user rules]**.

## Decision Drivers

* No cross-tenant read/write even with a repository bug (defence in depth).
* Public endpoints expose minimum data.
* Outbound fetches must not reach internal networks/metadata endpoints.
* Queue/workflow messages are untrusted input at consumers.
* Keys and sessions are revocable and stored hashed.

## Considered Options

* Option 1: STRIDE per trust boundary, mapped to an OWASP ASVS L2 checklist, with abuse cases turned into tests
* Option 2: Asset-centric risk register (data classes × adversaries), lighter and ad hoc
* Option 3: Defer threat modelling; rely on framework defaults plus periodic pen tests

## Decision Outcome

Decision pending — see Q-0141. Leading hypothesis: **Option 1** (confidence: medium). Baseline controls proposed for
adoption regardless of the final model: tenant predicate enforced in the repository layer *and* verified in tests
(negative cross-tenant tests per aggregate); API keys stored as peppered hashes with prefix lookup *(amended 2026-10-24: SHA-256 with no pepper, see the update below)*; SSRF guard (deny
RFC1918/link-local/loopback/metadata, re-resolve after redirect, size/time limits) shared by webhook delivery and link
crawl; strict CORS/CSRF for cookie sessions; consumers validate message schemas (zod/JSON-schema) and authorise by
tenant id carried in the message; public-space DTOs are separate types from internal ones.

### Consequences

* Good, because risks are enumerated per boundary and become CI tests.
* Bad, because up-front effort and ongoing maintenance of the model.
* Neutral: the DB-per-tenant option (ADR-0041) changes which controls are primary (physical isolation vs predicate).

### Confirmation

A `security/` test suite: cross-tenant matrix (every repository method × foreign tenant id must fail), SSRF corpus,
API-key brute-force/rate-limit, public-space field allowlist snapshot test. Threat model reviewed at each ADR
promotion touching a boundary.

## Pros and Cons of the Options

### Option 1: STRIDE + ASVS

* Good, because structured, standards-aligned, test-producing.
* Bad, because heavy for small scope if done exhaustively.

### Option 2: Asset-centric register

* Good, because lightweight.
* Bad, because easier to miss boundary-specific threats.

### Option 3: Defer

* Good, because zero up-front cost.
* Bad, because tenancy and SSRF defects are architectural and expensive to retrofit.

## Update — interview round 17 (2026-10-17)

No formal compliance driver (Q-0141.1; revisit via `plane-9y3` if one appears). Public spaces: **60 requests/min/IP with edge caching** (Q-0141.3). **SSRF guard on all server-initiated fetches** (R-S3).

## Update — owner decisions and spike results, round 27 (2026-10-18)

Threat-model inputs from spike 0004 (real platform): (1) the Access bypass for `/api`, `/auth` and `/live` is **case-insensitive**, so case variants reach the origin without Access (ADR-0050, Q-0050.7); (2) client **service-token secrets sent to a bypass path are forwarded to the origin**; (3) a client-supplied `Cf-Access-Jwt-Assertion` header is not stripped on bypass paths (the email header is); (4) `..;`, `..%2f`, `%5c` and `%252e` stay under the bypass prefix and reach the Worker literally; (5) domain fronting between the product and space hostnames is blocked by the edge (SNI/Host mismatch answers 403).

## Update — owner decision, round 27 (2026-10-18)

Two upstream defects are deliberately **not** reproduced (Q-0033.3, round 27): PQ-412 (any valid API key reads any project's attachment metadata, D-014) and PQ-413 (`cycle-issues/` moves other projects' or tenants' issues, D-015). Both are tenant-scope violations; the oracle diff gets an exception for each. The catalogue's other 155 entries default to `reproduce`.

## Update — spike 0006 results, round 27 (2026-10-18)

Sanitiser evidence from spike 0006: Plane's current sanitiser (nh3 0.2.18, ammonia 4.0.0) lets **9 of 125** executable test payloads survive (CSS `url(javascript:)`, `expression()`, `behavior:`, `-moz-binding`, `td background`, a callout `data-emoji-url`), all legacy-browser or CSS-injection class and probably not exploitable in evergreen browsers; a hardened Rust config closes all 125 and changes none of 195 editor documents. Not blocked by this: `style="position:fixed…"` (UI redress) and `id`/`name` clobbering remain allowed. The stance on hardening is Q-0012.2.

## Update — Q-0052.3, owner decision (2026-10-24)

The baseline control above, "API keys stored as peppered hashes with prefix lookup", is amended; the text above is left
as it was written. An API key is stored as the SHA-256 of a token of at least 256 random bits (`thrk_` and 64 hex
characters), with no pepper, and the lookup is by that digest (a unique index): the prefix is only what is shown again
in a listing. A pepper is for low-entropy secrets that a leaked hash could be guessed back from, and a key is not one;
passwords are argon2id with a salt of their own, and session tokens are plain SHA-256 for the same reason as keys
(2026-10-22). So a leak of the `api_keys` table does not yield a usable key either way, and a pepper would add a secret
and a rotation story to every target. The decision keeps a way out: `api_keys.hash_version` and the versioned
`api_key_hash` let a pepper be introduced later without invalidating a key (ADR-0052's 2026-10-24 note has the steps, the
query that shows progress and the caveats). Everything else in the baseline stands.

## Open Questions

* Q-0141 — threat model approach.
* Q-0141.1 — Compliance requirements (SOC2/ISO) that drive evidence/audit logging for the single-org deployment (relates to F-E06 audit logging)?
* Q-0141.2 — Public-space exposure: Access-gated, unauthenticated, or both (interplay with Q-0022)?
* Q-0141.3 — Rate-limit/abuse controls for unauthenticated endpoints (auth callbacks, public spaces).

## More Information

`apps/api` URL security tests; Open-EE `rbac`, `rebac-fine-grained`, `audit-logging`, `guests` specs on `woven/plan`. Related: [[ADR-0003]], [[ADR-0033]], [[ADR-0041]], [[ADR-0050]], [[ADR-0051]], [[ADR-0140]].
