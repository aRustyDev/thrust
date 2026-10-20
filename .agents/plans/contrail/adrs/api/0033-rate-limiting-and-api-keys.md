---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0033: Rate limiting and API keys

## Context and Problem Statement

Plane authenticates the public API with `X-Api-Key` (`APIToken`) and applies a rate limit controlled by the `API_KEY_RATE_LIMIT` env var; usage is logged in `APIActivityLog` [S]. In the new system the limiter must be a port (ADR-0002) implemented with Cloudflare primitives (KV is eventually consistent; DOs give strong counters; Workers has a rate-limiting binding) or Redis on the self-hosted set. API-key storage/hashing and migration of existing key values are covered by ADR-0052.

## Decision Drivers

* Preserve `X-Api-Key` semantics for v1 clients (ADR-0031).
* Accuracy vs cost and latency of the limiter.
* Both adapter sets must offer a comparable limiter.
* Per-key scopes may be wanted later (Open-EE RBAC).

## Considered Options

* Option 1: `RateLimiter` port; CF adapter uses the Workers rate-limiting binding; self-hosted uses Redis
* Option 2: `RateLimiter` port; CF adapter uses a Durable Object per key (exact counters); self-hosted uses Redis
* Option 3: KV-based approximate counters on CF; Redis self-hosted
* Option 4: Rely on Cloudflare edge rate limiting rules (WAF) only, no application limiter

## Decision Outcome

**Decision (accepted, round 15): Option 1** — `RateLimiter` port; the Cloudflare adapter uses the Workers rate-limiting binding (approximate, per-location), the self-hosted adapter uses Valkey/Redis; no key scopes in v1.

**Supported subset of `API_KEY_RATE_LIMIT`:** Plane takes a free-form string (e.g. `60/minute`); the Workers Rate Limiting binding only accepts a `period` of **10 or 60 seconds** and counts per Cloudflare location, so the limiter accepts only limits expressible per 10 s or per 60 s (e.g. `60/minute`), and config validation rejects other periods at startup. Verified against Cloudflare documentation during the round-25 review.

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 1** (confidence: low-medium) because a coarse per-key limit is sufficient for Plane's current semantics and the binding is the cheapest; use Option 2 if exact quotas become a requirement. Confirm that the binding's semantics (per-location, approximate) are acceptable.)

### Consequences

* Good, because low cost and no extra state to manage.
* Bad, because per-location and approximate enforcement differs from Redis's global counter, so contract tests must assert limits with tolerance.
* Neutral: Option 4 cannot differentiate API keys inside the application.

### Confirmation

Port contract test: N+1 requests in the window are rejected with the same status/headers on both adapters, within a documented tolerance; load test for limiter overhead.

## Pros and Cons of the Options

### Option 1: Workers rate-limiting binding

* Good, because cheapest, no state.
* Bad, because approximate and per-location.

### Option 2: DO per key

* Good, because exact and consistent.
* Bad, because a DO round trip per request and a hot-key bottleneck risk.

### Option 3: KV counters

* Good, because simple.
* Bad, because eventual consistency makes it inaccurate.

### Option 4: Edge rules only

* Good, because zero code.
* Bad, because not key-aware and not portable.

## Update — interview round 15 (2026-10-17)

Accepted. Keep `X-Api-Key` and Plane's rate-limit semantics; **no scopes in v1** (Q-0052.2); approximate per-location limits from the Workers rate-limiting binding are acceptable (Q-0033.1). Scopes can be added later via the `Authorizer`.

## Update — review fixes, round 25 (2026-10-18)

Added the supported subset of `API_KEY_RATE_LIMIT` (10 s or 60 s periods, per-location) and replaced the Redis reference for the self-hosted set with Valkey (round 20).

## Update — spike results, round 26 (2026-10-18)

Facts for the Rust implementation (Q-0033.2). From the pinned oracle (spike 0008): only a **missing** key returns 401, while an invalid, expired or inactive key returns **403**; the 429 body is `{"error_code":5900,"error_message":"RATE_LIMIT_EXCEEDED"}` with a `Retry-After` header, and `X-RateLimit-Remaining` counts down (4, 3, 2, 1) under the oracle's test limit. From spike 0001 (`[tested]` locally): the Workers Rate Limiting binding is native in workers-rs (`env.rate_limiter`; limit 3 per 10 s gave `[true, true, true, false, false]`), and the spike's per-user 429 with `Retry-After` used a fixed-window counter in a SQLite Durable Object, about $0.60/month at 5 M requests (`research/0004`, section 9). The binding's real per-location behaviour is untested (Stage 2 step S2-6).

## Update — owner decisions and spike results, round 27 (2026-10-18)

Owner decision, round 27 (Q-0033.2): reproduce the oracle's behaviour, including the 403-for-an-invalid-key and the 429 body described above; document it in the quirks catalogue (`research/0007`).

## Update — review round 29 (2026-10-19)

Per FR-05 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now, all five cuts), rate limiting is the Workers Rate Limiting binding on Cloudflare and an in-process counter natively. D-007 accepts the binding's approximate, per-location limits, so the exact Durable Object limiter is not required. The Durable Object limiter (`DoRateLimiter`, `crates/thrust-adapters-cloudflare/src/lib.rs:193-223`, wired at `crates/thrust-worker/src/lib.rs:94`; one Durable Object hop on every create) is removed from the API Worker (issue B9). Its Durable Object code is kept only as the template for the P1 collab Durable Object (FR-05). The native counter is the fixed-window counter the server already wires (`MemoryRateLimiter`, `crates/thrust-server/src/main.rs:84`), which is enough in the single-process self-hosted shape (ADR-0070, round 29). Valkey is dropped from P0 and P1 (ADR-0082, round 29).

This supersedes the self-hosted half of the round-15 Decision ("the self-hosted adapter uses Valkey/Redis"), the round-25 note's Valkey reference, the round-26 note's Durable Object fixed-window counter as the Cloudflare limiter, and the round-28 statement in ADR-0006 that "the Durable Object limiter stays for exact limits". The Cloudflare half of the round-15 Decision (Option 1, the binding) and the supported subset of `API_KEY_RATE_LIMIT` (10 s or 60 s periods) stand. The Confirmation stands: the contract test asserts limits with a documented tolerance, because the binding is approximate (limit 3 allowed 4, review §4).

## Open Questions

* Q-0033 — limiter placement and semantics.
* Q-0052 — API-key storage, hashing and migration.

## More Information

Related: [[ADR-0002]], [[ADR-0031]], [[ADR-0052]], [[ADR-0082]]. Paths: `apps/api/plane/api` (rate limit settings), `APIToken`, `APIActivityLog`.
