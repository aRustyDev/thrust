---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0052: Sessions and API keys

## Context and Problem Statement

Plane uses Django server-side sessions (cookie, CSRF via `BaseSessionAuthentication`, session store in Redis/DB) for the UI, and `APIToken` (`X-Api-Key`, per-workspace, `user_type` Human/Bot, `API_KEY_RATE_LIMIT`, requests logged to `APIActivityLog`) for `/api/v1` **[S]**. The new backend must provide equivalents behind `SessionStore`/`Cache`/`RateLimiter` ports (ADR-0082) for both adapter sets. Concerns: KV is eventually consistent (revocation lag), DO gives strong consistency at a cost, API keys must be hashable at rest, and nothing is imported (Q-G3), so no legacy-token migration is needed.

## Decision Drivers

* Revocation latency (sign-out, deactivated user, key revoked) must be bounded and small.
* Secrets at rest: store only a hash of API keys; show once.
* Keep `X-Api-Key` wire format and `/api/v1` auth semantics (ADR-0031).
* CSRF protection for cookie-authenticated mutations; SameSite and domain behind Access.
* Per-key rate limiting accuracy (Q-0033).

## Considered Options

### Sessions
* S1: Opaque session id in an HttpOnly cookie; session record in DB (source of truth) with `SessionStore` cache (KV/Redis) for fast lookup
* S2: Stateless signed cookie (JWT/PASETO) with short TTL + refresh
* S3: Session state in a Durable Object per user/session (strongly consistent) — self-hosted twin emulates with Redis

### API keys
* K1: High-entropy random token with visible prefix; store SHA-256 hash + metadata (workspace, user_type, last_used, expires; no scopes in v1)
* K2: Store token plaintext as Plane does (verify in repo) for exact parity
* K3: Signed self-describing token (macaroon/JWT) validated without lookup

## Decision Outcome

**Decision (accepted, rounds 15 and 25): sessions — S1 without a cache at P0** (opaque session id in an HttpOnly cookie; the database row is authoritative; 30-day lifetime; sign-out-everywhere; CSRF via double-submit token); **API keys — K1** (high-entropy token with a visible prefix; SHA-256 hash stored; shown once; no scopes in v1).

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypotheses: **S1** (medium) and **K1** (high).)

* **S1**: DB row is authoritative and read on every request (revocation = row update, effective immediately); **no KV/Valkey cache at P0** (round 25; consistent with ADR-0051); CSRF via double-submit token (as Plane's `CSRFTokenEndpoint` **[S]**). Access JWT (ADR-0050) is verified independently on the CF adapter once that phase lands.
* **K1**: tokens shown once at creation; lookup by hash (indexed); nothing is imported (Q-G3); `last_used_at` updated asynchronously (avoid write per request). Rate-limit counters via `RateLimiter` port (CF: Workers rate-limit binding or DO for exact counts; self-hosted: Redis) — placement decided in Q-0033.
* Bot users and per-workspace scoping preserved; **no key scopes in v1** (Q-0052.2; scopes can be added later via the `Authorizer`).

### Consequences

* Good: immediate revocation; no plaintext secrets; port keeps both adapter sets equivalent.
* Bad: a DB read per request and a hash lookup per API call (acceptable at ≤50 users; a short-TTL key→principal cache may be added later, with revocation checked against the DB).
* Neutral: SCIM/deprovisioning (ADR-0053) must revoke sessions and keys of deactivated users.

### Confirmation

Tests: revoke key → 401 on the next request; deactivated user's session rejected; hash-only storage (no plaintext column); CSRF failure on mutation without token; `RateLimiter` contract test on both adapters.

## Pros and Cons of the Options

### S1 DB + cache
* Good: revocable, auditable, adapter-neutral. Bad: extra lookup.
### S2 Stateless cookie
* Good: no storage, very fast. Bad: revocation hard until expiry; token size; rotation complexity.
### S3 DO per session
* Good: strongly consistent revocation. Bad: DO per session overhead/cost; not portable without emulation.
### K1 Hashed
* Good: breach-resistant; standard practice. Bad: keys cannot be recovered after creation (by design; no import, Q-G3).
### K2 Plaintext (parity)
* Good: exact parity. Bad: insecure at rest; no reason to carry forward.
### K3 Self-describing
* Good: no lookup. Bad: revocation requires denylist; secret rotation complexity.

## Update — interview round 15 (2026-10-17)

Accepted. **30-day sessions with sign-out-everywhere**, API keys stored hashed, no key scopes in v1 (Q-0052.1, Q-0052.2).

## Update — review decisions, round 25 (2026-10-18)

Sessions: **database row is authoritative and there is no KV/Valkey cache at P0** (supersedes the TTL ≤ 60 s cache in S1). Nothing is imported (Q-G3), so import-era text (re-hashing legacy tokens, 'imported legacy key authenticates' test) is void; no key scopes in v1.

## Update — review fixes, round 25 (2026-10-18)

Removed import-era text (legacy-token re-hash, 'imported legacy key authenticates' test), removed the `scopes` metadata field, and rewrote S1 as DB-authoritative with no cache at P0. Decisions unchanged.

## Open Questions

* Q-0052 — session and key mechanics.
* Q-0033 — rate-limit placement and API key semantics.
* Q-G3 — answered: start fresh, no token migration.
* Q-0052.1 — Session lifetime/idle timeout policy and "sign out everywhere" requirement.
* Q-0052.2 — Do API keys need scopes (read-only keys) in v1, or exact Plane parity?

## More Information

[[ADR-0031]], [[ADR-0050]], [[ADR-0053]], [[ADR-0082]]. Plane: `apps/api/plane/authentication/session.py`, `app/middleware/api_authentication.py`, `db/models/api.py` (`APIToken`, `APIActivityLog`).
