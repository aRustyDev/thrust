---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0082: `Cache` / `SessionStore` / `RateLimiter` ports — KV (+ Durable Objects for strong state) vs Redis/Valkey

## Context and Problem Statement

Plane uses Redis/Valkey 7.2 for Django cache, sessions, Celery results and the live server's Hocuspocus Redis extension **[S]**; API keys carry a per-key
rate limit (`API_KEY_RATE_LIMIT`) **[S]**. The interview ([I] round 3) listed "Cache/session: KV ↔ Redis/Valkey" as an accepted adapter pair. Workers KV is
**eventually consistent**, which is wrong for some of what Redis does implicitly (sessions revocation, counters, locks, one-time codes). The decision is therefore
three ports, not one, so each can pick the right CF primitive.

## Decision Drivers

* Correctness first: magic codes, session revocation, rate limits and locks cannot tolerate stale reads.
* Cheap, globally fast reads for genuinely cacheable data (instance config, feature flags, permission snapshots with TTL).
* Same port contracts on self-hosted (Redis/Valkey), where everything is strongly consistent.

## Considered Options

* Option 1: Three ports — `Cache` (KV adapter), `SessionStore` and `RateLimiter` (DO adapters), self-hosted: all on Redis/Valkey
* Option 2: One `KeyValue` port, KV for everything on CF
* Option 3: One `KeyValue` port, DO-backed for everything on CF
* Option 4: Signed/stateless sessions (JWT/cookie) and Workers rate-limit binding; KV only for cache

## Decision Outcome

Chosen option: **Option 1**, because it makes the consistency requirement explicit per use. Source: interview round 3 (KV↔Redis pair), refined here for consistency.

**Revised decision (round 25) — read this before the table below.** `Cache` stays a KV/Valkey port for genuinely cacheable data. **`SessionStore` is database-authoritative with no cache at P0 on both adapter sets (no Durable Object session class)**; the "Sessions" row below, its DO/Redis adapters and the negative "KV SessionStore fails the suite" test are **superseded**. One-time codes and `RateLimiter` keep the classification below as the *design intent* but their concrete backing (DB table vs DO vs Workers rate-limit binding with its 10 s / 60 s period constraint) is decided in ADR-0033 / ADR-0052; no DO class is introduced for them at P0.

Consistency classification (binding on adapters; Sessions row superseded as stated above):

| Use | Needs | CF adapter | Self-hosted |
|---|---|---|---|
| Config/flag/permission **cache** with TTL | eventual OK | KV (`cacheTtl` ≥ 30 s, default 60 s) | Redis |
| Rendered/aggregated read cache | eventual OK | KV or Cache API | Redis |
| **Sessions** (create, rotate, revoke) | strong | DO (one object per session or per user) — or stateless signed cookie + DO revocation list (Option 4 hybrid) | Redis |
| **One-time codes** (magic code, email-change code) | strong, single-use | DO | Redis (`GETDEL`) |
| **Rate limits** (`API_KEY_RATE_LIMIT`, login attempts) | strong-ish counters | DO counter (or Workers rate-limiting binding if accepted as approximate) | Redis `INCR`+TTL |
| **Locks / dedupe keys** (idempotency) | strong | DO or DB | Redis `SET NX` |

### Consequences

* Good: KV is global and fast for reads. Documented behavior: writes visible immediately in the same location but **up to 60 s (or the `cacheTtl`) elsewhere**; concurrent writes to one key
  are last-write-wins; ≤ 1,000 external-service operations per Worker invocation; bulk write API up to 10,000 pairs (CF KV docs).
* Good: strong-consistency needs are served by DOs, which are single-threaded with transactional storage.
* Bad: three adapters per set; DO-per-session design needs care (hot users, fan-in cost).
* Bad: KV cache invalidation after config change is bounded-stale (≤ 60 s) — acceptable for flags/config, **not** for permission revocation (see Q-0082.1).

### Confirmation

* Contract suite (ADR-0121): `SessionStore` revoke is visible to the very next request from a different location (simulated) — must pass on the DB-backed adapter (round 25); `Cache` contract explicitly allows staleness ≤ TTL.
* Negative test: an adapter that serves `SessionStore` reads from a cache (KV/Valkey) without a DB revocation check fails the suite.

## Pros and Cons of the Options

### Option 1: Three ports

* Good: right tool per consistency need; portable semantics documented. * Bad: more surface to build and test.

### Option 2: KV for everything

* Good: simplest on CF. * Bad: stale session revocation, double-use magic codes, racy counters — security bugs.

### Option 3: DO for everything

* Good: one strongly consistent store. * Bad: wasteful for hot read-mostly caches; per-key DO overhead; no global read replication like KV.

### Option 4: Stateless sessions + rate-limit binding

* Good: fewer moving parts; scales. * Bad: revocation needs a server-side list anyway; Workers rate-limit binding is per-location approximate; self-hosted equivalence needs care.

## Update — interview round 20 (2026-10-18)

Self-hosted cache/session adapter: **Valkey** (Q-0082.2).

## Update — review decisions, round 25 (2026-10-18)

**Session-store mandate withdrawn.** The owner chose **database-authoritative sessions with no cache at P0** (consistent with ADR-0051: no authorization cache). The earlier rule that `SessionStore` MUST be a Durable Object and that a KV-backed store 'fails the suite' no longer applies; `SessionStore` is implemented over the database port on both adapter sets, so there is no DO session class (also resolves the DO-class ownership conflict with ADR-0090). KV (Cloudflare) / Valkey (self-hosted) remain available for caching later, with revocation checked against the DB. One-time codes and rate limiting are decided separately (rate limiting: ADR-0033).

## Update — review fixes, round 25 (2026-10-18)

The decision body now carries a **Revised decision (round 25)** paragraph: the session-store Durable Object mandate is withdrawn (sessions DB-authoritative, no cache at P0). Port sketches are Rust traits in `thrust-ports` (ADR-0006).

## Update — spike results, round 26 (2026-10-18)

Note for readers of spike 0001: its Cloudflare adapter set uses a KV-backed session store because KV was the quickest native session binding to exercise. It is **not** the accepted design: sessions are DB-authoritative with no cache at P0 (round 25). KV's eventual consistency cannot show up locally and was not observed; anything needing read-your-writes must not use KV (a DO for strong counters, as above).

## Update — review round 29 (2026-10-19)

Per FR-03 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now), the spike's KV session store is removed. `KvSessionStore` (`crates/thrust-adapters-cloudflare/src/lib.rs:65-94`) is deleted from the Cloudflare set, which ends the round-26 exception above. `SessionStore` is a repository over the `Database` port on both adapter sets (a session row, `revoked_at`, `last_seen_at` updated lazily), and it returns `Principal { user_id }` only (ADR-0052, round 29; D-003). The memory adapter may remain for tests. This supersedes nothing in the round-25 revised decision; it removes the last code that did not follow it.

Per FR-05 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now, all five cuts), Valkey is dropped from P0 and P1. There is no session cache (round 25) and no authorization cache (ADR-0051), so Valkey has no job; this supersedes the round-20 note (Valkey as the self-hosted cache and session adapter, Q-0082.2) and the table's self-hosted Redis column. A `Cache` port is added only when a measured hot path needs it; until then there is no `Cache` port (`crates/thrust-ports/src/lib.rs` defines none) and no KV cache adapter. Rate limiting is the Workers Rate Limiting binding on Cloudflare (approximate and per location, D-007) and an in-process counter natively (ADR-0033, round 29). The Durable Object limiter (`DoRateLimiter`, `crates/thrust-adapters-cloudflare/src/lib.rs:193-223`) is removed from the API Worker, which supersedes the table's "DO counter" for rate limits; its Durable Object code is kept only as the template for the P1 collab Durable Object (FR-05).

## Open Questions

* Q-0052 — **answered (rounds 15, 25)**: DB-authoritative sessions, no cache at P0; API-key hashing in ADR-0052.
* Q-0033 — **answered (round 15)**: keep Plane's semantics; approximate Workers rate-limit binding acceptable.
* Q-0082.1 — **answered (rounds 24–25)**: no authorization or session cache at P0; revocation immediate over HTTP.
* Q-0082.2 — **answered (round 20)**: Valkey for the self-hosted `Cache` adapter. (superseded round 29, FR-05)

## More Information

* Plane: `apps/api/plane/settings/redis.py` (cache/sessions), compose service `plane-redis` (valkey 7.2.11), `API_KEY_RATE_LIMIT`.
* CF: KV write guidance <https://developers.cloudflare.com/kv/api/write-key-value-pairs/>, read `cacheTtl` <https://developers.cloudflare.com/kv/api/read-key-value-pairs/>.
* Related: [[ADR-0002]], [[ADR-0033]], [[ADR-0052]], [[ADR-0121]].
