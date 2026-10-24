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

## Update — owner decisions on the #38 review (2026-10-24)

The owner decided the three questions that PR #76 (issue #38) left open: Q-0052.3 here, and Q-0050.8 and Q-0050.9 in
ADR-0050. The first two change behaviour and are built (bead `plane-byd.11.6`, the PR that carries this note); the third
is recorded for M6.

**K1 stays plain SHA-256, with no pepper, and has an escape hatch (Q-0052.3, answered).** A key is `thrk_` and 256 random
bits, so a leaked digest cannot be reversed with a pepper or without one: there is nothing low-entropy to guess back. A
pepper would add a secret, and a rotation story for it, to every target (the Worker and the native server each need to
read it) for no gain. Sessions took the same decision on 2026-10-22, and a pepper is for low-entropy secrets, which here
means passwords, and those are argon2id with a salt of their own. ADR-0141's "peppered hashes" for keys is amended by a
note there. The hatch exists so that this can change without a flag day: **a pepper can be introduced later and no key is
invalidated.** What is built:

- `api_keys.hash_version smallint NOT NULL DEFAULT 1`, checked positive by the named constraint
  `api_keys_hash_version_positive` (migration `0007_api_key_hash_version.sql`; `0006` is left to the SQLite adapter).
  Every key made before the column existed is version 1. (A CHECK written inline with `ADD COLUMN IF NOT EXISTS` is added
  again on every run of an idempotent migration, three runs, three checks, which is why it is a statement of its own.)
- `thrust_core::api_key_hash(version, token)`: version 1 is `hash_token`, the plain SHA-256, and the only scheme that
  ships. `API_KEY_HASH_CURRENT` and `API_KEY_HASH_SUPPORTED` are the shipped constants, and `CoreConfig` carries them as
  `api_key_hash_current` and `api_key_hash_supported` (code, not an environment variable: a scheme is a code change, and a
  stray variable must not be able to change how keys are hashed).
- `authenticate_api_key` looks the key up under every version the process reads, the current one first, and accepts a row
  only when it is stored under the version whose digest found it. A key that is found under another version than the
  current one is moved to it after the response, by the task that stamps `last_used_at` (`Background`), through
  `Database::rehash_api_key(id, from_version, new_hash, new_version)`: a compare-and-set that does nothing when the row is
  no longer at `from_version`. A failed move is ignored (the key keeps working and is moved by its next use).
  `create_api_key` stores the current version.
- Proved with a stand-in second scheme that exists only under `cfg(test)`: version 2, an HMAC-SHA256 under a fixed public
  test pepper (checked against RFC 4231). No build that ships has a second scheme or a pepper.

To introduce a pepper (or any second scheme):

1. Generate the pepper (at least 256 random bits) and put it where every target can read it at startup: a Worker secret,
   an environment variable natively. It is a secret with the durability of the database: if it is lost, every key moved to
   version 2 stops authenticating. Never commit it.
2. Add the scheme as a new arm of `api_key_hash` (HMAC-SHA256 keyed by the pepper, bound at startup; the function is pure
   today), under version 2. Leave version 1 as it is. The `cfg(test)` stand-in is the template for the tests.
3. Release 1: `API_KEY_HASH_SUPPORTED = [1, 2]` with `API_KEY_HASH_CURRENT` still 1. Every instance can now read both
   versions and nothing is written under 2.
4. Release 2, only after every instance runs release 1: `API_KEY_HASH_CURRENT = 2`. New keys are stored at version 2 and
   existing ones move as they are used, one background write each.
5. Watch the move with `SELECT hash_version, count(*) FROM api_keys GROUP BY 1`. To see the keys that still matter, add
   `WHERE revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())`.
6. **Idle keys** are never moved: a key that nobody uses keeps its version 1 digest, and keeps working for as long as
   version 1 is supported.
7. **Dropping version 1 is a deliberate act that invalidates the keys still at version 1.** Remove 1 from
   `API_KEY_HASH_SUPPORTED` only when the query shows nothing live is left there, or after telling their owners: those
   keys then get 403 like any invalid key, and the owner makes a new one.

**Rollback caveat.** Code that no longer knows version 2 cannot authenticate a version 2 key (the digest cannot be
recomputed, so the key looks unknown) and cannot move it back. After step 4, the safe rollback is to set
`API_KEY_HASH_CURRENT` back to 1 and keep version 2 in the supported set: the keys found at version 2 then move back to 1
as they are used. Going back to a build that predates the scheme strands every key that was moved. A pepper is not edited:
a rotation is a new version (3), with the same steps.

**Q-0050.8: option A is built and is not enforced while the stock frontend exists (answered).** The design is the one
ADR-0052 describes: `GET auth/csrf`, the double-submit cookie, and `X-CSRFToken` on a cookie-authenticated mutation. But
the stock Plane React app sends the token only on its auth form posts and on `set-password` (`research/0012` §5), so
enforcing it on JSON mutations would break that app. The frontend is going to be custom, and enforcement is the target.
Until then there is a switch.

- `CoreConfig::csrf_enforce`. Core's default is `true`, the rule as this ADR wrote it. The composition roots read
  `THRUST_CSRF_ENFORCE` (the native server's environment, the Worker's var, declared in `wrangler.toml` and
  `wrangler.memory.toml`) and default it **off**. The contract targets and Stage 2 run with it **on**
  (`scripts/contract.sh`, `scripts/worker-dev.sh`, `wrangler.stage2.toml`), and `/_info` (test routes) reports
  `csrf_enforce`, so that `scripts/contract.sh` can refuse a pass whose server says otherwise.
- Off changes one thing: a cookie-authenticated mutation is accepted without the token, whatever is sent in its place.
  The cookie, `GET auth/csrf`, `csrf_ok` and the exemption of a bearer token and an API key are as before, and so is the
  order of the checks (a dead session is 401 first).
- **The interim posture, plainly.** While it is off, the only cookie-side protection against a forged cross-site request
  is `SameSite=Lax`, `HttpOnly`, `Secure` and the `__Host-` prefix. That is the same posture as Plane, whose REST API
  disables Django's CSRF check (`apps/api/plane/authentication/session.py:10`). `Lax` is a browser behaviour (it does not
  send the cookie on a cross-site POST, PUT, PATCH or DELETE); it does not help against a request from the same site (a
  sibling subdomain; the `__Host-` prefix stops one from planting a cookie, not from sending requests) or in a browser
  that does not implement it.
- **The flip criterion.** Turn it on when the custom frontend replaces the stock one: when no cookie-authenticated
  mutation is made by a client that does not send the token. The flip is `THRUST_CSRF_ENFORCE=on` in the Worker's vars
  and the server's environment, with no code change, and it is an explicit item of PLAN.md's M4 row. The main contract
  pass runs the enforcing behaviour on every target on every pull request, so it stays tested while the production
  default is off.
- Tested in both modes: the core credential matrix has an enforcement dimension, the HTTP credential table runs in the
  main pass (on) and in the new `csrfoff` pass (off, the target started with no setting, so the pass also shows the roots'
  own default). Controls, all shown failing: `mutant_csrf_never` (switch ignored when on, killed by the main pass),
  `mutant_csrf_always` (ignored when off, killed by `csrfoff`), `mutant_skip_csrf` as before, and the guard refusing a
  server that does not match its pass.

**Q-0050.9 (for M6, not built).** A deployed Worker reaches the OIDC issuer through a Workers VPC service over a tunnel
that can reach `id.auth.woven`, as for Postgres (ADR-0094); see ADR-0050's note.

**Confirmation, amended.** "CSRF failure on mutation without token" holds while enforcement is on, which is the core's
default and every contract target's setting; the production default is off until the flip above.

## Open Questions

* Q-0052 — session and key mechanics.
* Q-0033 — rate-limit placement and API key semantics.
* Q-G3 — answered: start fresh, no token migration.
* Q-0052.1 — Session lifetime/idle timeout policy and "sign out everywhere" requirement.
* Q-0052.2 — Do API keys need scopes (read-only keys) in v1, or exact Plane parity?
* Q-0052.3 — Should API-key hashes carry ADR-0141's pepper? **Answered (owner, 2026-10-24): no.** Plain SHA-256 of a 256-bit token, with the `hash_version` hatch so a pepper can be added later without invalidating a key (the 2026-10-24 update above).

## More Information

[[ADR-0031]], [[ADR-0050]], [[ADR-0053]], [[ADR-0082]]. Plane: `apps/api/plane/authentication/session.py`, `app/middleware/api_authentication.py`, `db/models/api.py` (`APIToken`, `APIActivityLog`).
