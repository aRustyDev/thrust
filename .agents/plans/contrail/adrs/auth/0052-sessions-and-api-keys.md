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

Tests: revoke key → 403 on the next request (corrected from 401 by issue #38: the oracle answers 403 for an invalid, expired, revoked or inactive-owner key and 401 only for a missing one, ADR-0033 round 27; see the 2026-10-23 issue #38 update below); deactivated user's session rejected; hash-only storage (no plaintext column); CSRF failure on mutation without token; `RateLimiter` contract test on both adapters.

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

## Update — review round 29 (2026-10-19)

Per FR-03 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now), a session resolves to a user only, and the session store is a repository over the database. This makes the round-25 S1 decision (the database row is authoritative, no cache at P0) concrete. It supersedes the spike's session shape, not the Decision.

* `Principal { user_id }` only. Today it is `Principal { user_id, workspace_id }` (`crates/thrust-contracts/src/lib.rs:81-86`), which binds one workspace to a session and cannot serve a user who is in several workspaces. The workspace is resolved per request from the URL slug inside `authorize` (ADR-0051, round 29).
* `SessionStore` becomes a repository over the `Database` port: one row per session, a `revoked_at` column, and `last_seen_at` updated lazily rather than on every request. As S1 already says, revocation is a row update, effective on the next request.
* `KvSessionStore` (`crates/thrust-adapters-cloudflare/src/lib.rs:65-94`: Workers KV, with a 60 s minimum TTL on an eventually consistent store) is deleted from the Cloudflare set (ADR-0082, round 29; D-003). The `SessionStore` trait is kept only if the memory adapter is still wanted for tests.

Per FR-05 (same review and §5 table), K1's rate-limit placement ("CF: Workers rate-limit binding or DO for exact counts; self-hosted: Redis") is superseded: the counters are the Workers Rate Limiting binding on Cloudflare (approximate, D-007) and an in-process counter natively. There is no Durable Object limiter and no Redis (ADR-0033 and ADR-0082, round 29).

## Update — issue #36 (2026-10-23)

Issue #36 built the FR-03 session shape from the round-29 note. The facts, with their locations on the branch that
closes #36:

- **Sessions are `Database` rows.** Migration `0004_sessions.sql` creates `sessions`: `token_hash` (`bytea`, the
  key, checked to be 32 bytes), `user_id`, `created_at`, `expires_at`, `revoked_at` and `last_seen_at`. There is no
  plaintext column and no `workspace_id`, because a session belongs to a user (a directory table, ADR-0041).
  `Database` gained `session`, `touch_session`, `revoke_session` and `insert_session`
  (`crates/thrust-ports/src/lib.rs`).
- **The `SessionStore` trait is not kept.** The memory `Database` serves the tests, so `SessionStore`,
  `MemorySessionStore`, `KvSessionStore`, `CacheError` and the `SESSIONS` KV binding in both `wrangler*.toml` files
  are deleted. The native server's sessions now survive a restart on the `postgres` set and are shared between
  processes.
- **Hashing: SHA-256, no pepper** (owner, 2026-10-22). `thrust_core::hash_token` hashes the token before any call, so
  no adapter sees one. A pepper protects low-entropy secrets that a leaked hash could be guessed back from, and a
  session token is not one: the decision depends on sign-in (#38) minting at least 128 bits of CSPRNG output. That is
  a requirement on #38, not something this issue can check, because the only tokens today are the test seed's.
  ADR-0141's "peppered hashes" are for API keys (K1), which this does not decide.
- **Lifetime and bookkeeping.** `CoreConfig.session_ttl_secs` is 30 days from creation. `authenticate` rewrites
  `last_seen_at` only when it is at least `session_touch_after_secs` (one hour) old, and a failed touch does not fail
  the request.
- **One answer for a bad session.** A token whose row is missing, revoked or expired is `Unauthenticated` (401),
  and the lookup does not say which.
- **What is not built.** Sign-in, sign-out and sign-out-everywhere belong to #38; `sessions_user` is the index that
  sign-out-everywhere will use. Until then `POST /internal/revoke-session` (test routes and the admin token) revokes
  a session, and `/internal/seed` creates them.

## Update — issue #38, the auth slice (2026-10-23)

Spike 0003 built S1 and K1 as the Rust auth slice and measured them (`research/0012-auth-slice.md`). The outcome, with
the places it differs from the text above:

- **Correction: a revoked key is 403, not 401.** The Confirmation's "revoke key → 401" was wrong. Plane answers 403 for
  an invalid, expired, revoked or inactive-owner key, and 401 only for a missing (or empty) `X-Api-Key` (ADR-0033, round
  27), and the contract tests assert exactly that: `a_revoked_key_is_rejected_on_the_next_request_with_403` (with an
  unrevoked key at 200 beside it), `an_expired_key_and_a_deactivated_users_key_are_rejected_with_403`, and a generated
  matrix of 126 requests (credential kind, state, method, CSRF-token position) against a status table. The lookup does
  not say which of the four failed.
- **Sessions (S1, no cache).** Sign-in (`POST /api/_slice/auth/sign-in/email`) mints a fresh token every time (256 random
  bits, hex) and a row holding only its SHA-256; the row is joined to `users` on every request, so a session of a user
  who is deactivated, or who has no row, is not live (fail closed). Sign-out revokes one session, sign-out-everywhere
  all of the user's (`sessions_user`). The cookie is `__Host-thrust_session` with `Secure; HttpOnly; SameSite=Lax;
  Path=/`, no `Domain`, `Max-Age` 30 days; `Lax` and not `Strict` because the OIDC callback is a top-level navigation
  whose cookie must reach the next page.
- **CSRF (double submit).** `GET /auth/csrf` returns a fresh 128-bit token as JSON (Plane's `get-csrf-token`) and sets it
  as `__Host-thrust_csrf` (same attributes, `HttpOnly` as in Plane); a cookie-authenticated mutation must send it back in
  `X-CSRFToken`, compared in constant time, or it is 403 `csrf_failed`. A bearer token and an API key are not ambient
  credentials and need no token; a safe method needs none; a dead session is 401 before the token is looked at. K3's gap
  (the stock React app sends no token on its JSON calls) is recorded in the research note, not decided (Q-0050.8).
- **Passwords: argon2id at OWASP's minimum** (m = 19,456 KiB, t = 2, p = 1; salt from the `Rng` port; the cost is read
  from the hash, so raising it locks nobody out). **Measured on a deployed Worker: p50 45 ms and p99 78 ms of CPU at the
  minimum, and a p99 of at most 93 ms across OWASP's five equivalent configurations, against the default 30 s limit.**
  An unknown email costs exactly one unit of hash work, like a wrong password (counted in a test, not timed), and every
  refusal is one 401 with one body. A sign-in limiter per normalised email (10 a minute by default) runs before any
  hashing.
- **API keys (K1).** `thrk_` and 256 random bits; the first nine characters are the visible prefix; only the SHA-256 is
  stored (`api_keys`, no plaintext column, pinned by a test of the column list); shown once, in the creation response.
  **No pepper:** the token is 256 random bits, so there is nothing to guess or stretch (the same reasoning as the
  2026-10-22 decision for sessions), which differs from ADR-0141's "peppered hashes" for keys (Q-0052.3). `X-Api-Key` is
  accepted after a bearer token and before the cookie; `last_used_at` is written through `Background`, at most once an
  hour per key. Creating, listing and revoking keys, and signing out, need a session: a key cannot mint keys or end
  sessions.
- **Controls.** `scripts/mutants.sh` plants `ignore_revoked` (it now breaks keys as well as sessions), `skip_csrf` and
  `unverified_email`; all seven mutants are killed.

## Open Questions

* Q-0052 — session and key mechanics.
* Q-0033 — rate-limit placement and API key semantics.
* Q-G3 — answered: start fresh, no token migration.
* Q-0052.1 — Session lifetime/idle timeout policy and "sign out everywhere" requirement.
* Q-0052.2 — Do API keys need scopes (read-only keys) in v1, or exact Plane parity?
* Q-0052.3 — Should API-key hashes carry ADR-0141's pepper? Issue #38 stores a plain SHA-256 (256-bit tokens). Open, owner asmith.

## More Information

[[ADR-0031]], [[ADR-0050]], [[ADR-0053]], [[ADR-0082]]. Plane: `apps/api/plane/authentication/session.py`, `app/middleware/api_authentication.py`, `db/models/api.py` (`APIToken`, `APIActivityLog`).
