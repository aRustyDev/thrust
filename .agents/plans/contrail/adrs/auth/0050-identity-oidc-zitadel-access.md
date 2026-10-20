---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0050: Identity — OIDC (Zitadel) + Cloudflare Access outer gate + API keys

## Context and Problem Statement

Plane CE authenticates with Django session cookies via a pluggable `Adapter` pipeline (email/password, magic code, Google/GitHub/GitLab/Gitea OAuth); there is **no OIDC, SAML or SCIM** in CE, and no central provider registry **[S]**. The woven environment uses Zitadel (`id.auth.woven`) as IdP and Cloudflare Access/WARP as the network perimeter **[O]**. The interview chose the identity posture below **[I, round 2: "OIDC (Zitadel) + Cloudflare Access, plus API keys"]**.

## Decision Drivers

* Match the woven environment (Zitadel SSO, Access, WARP-only hostnames).
* Machine clients need non-interactive credentials (`X-Api-Key`, public `/api/v1`, ADR-0031).
* Identity logic must be a port (`Identity`) with the same behaviour on the self-hosted adapter set (ADR-0002); Access is a CF-adapter concern.
* Keep the door open for other providers/passwords (tiering Q-0050.3).

## Considered Options

* Option 1: OIDC (Zitadel) as the identity source **+** Cloudflare Access as outer gate **+** API keys for machines
* Option 2: Full Plane auth parity (email/password, magic code, Google/GitHub/GitLab/Gitea) plus OIDC and API keys
* Option 3: Cloudflare Access only — trust the `Cf-Access-Jwt-Assertion` JWT as the sole identity source, no in-app login

## Decision Outcome

Chosen option: **Option 1**, because it was selected in the interview as the recommended posture for this environment and avoids carrying password/magic-code infrastructure by default. Specifics:

* **Generic OIDC relying party** in the new backend (authorization-code + PKCE, discovery via `.well-known/openid-configuration`, `sub` → provider id, **verified email required** — the same guard Plane added after GHSA-7j95-vh8g-f365 **[O]**). Issuer and client config come from `AppConfig`/secrets (ADR-0092), not hard-coded.
* **Cloudflare Access** validates user traffic at the edge; the API Worker additionally verifies the Access JWT (`aud`, `iss`, signature via team JWKS) when running on the CF adapter set, so a misrouted request cannot bypass the gate. Self-hosted adapter: Access verification is a no-op adapter; OIDC remains.
* **API keys** (`X-Api-Key`) remain the credential for `/api/v1` and machine clients; storage/rotation in ADR-0052.
* Other Plane providers are **not** in the default scope; each is an optional `Identity` adapter tiered in FEATURES.md (F-A03…F-A05) via Q-0050.3.
* User/Account model follows Plane's `User` + `Account(provider, provider_account_id)` shape for familiarity; **nothing is imported** (Q-G3, ADR-0042), so the final schema may follow the chosen identity implementation (spike 0003, Q-0050.6).

### Amendment A1 — interview round 4 (2026-10-17): framework and phasing

The end state above (OIDC + Access + API keys) stands, but the owner refined **how and in what order** **[I, round 4]**:

* **Framework: BetterAuth** is the intended basis for in-app authentication (sessions, credential, OIDC/generic-OAuth, API-key plugins). It must still sit behind the `Identity`/`SessionStore` ports (ADR-0002) — fit is unproven, see **Q-0050.4** (spike) and Q-0050.6 (schema mapping). (Superseded in round 29, FR-16: BetterAuth's HTTP shapes only; the implementation is the Rust auth slice. See the round-29 update.)
* **Phasing:** (1) **user + password — the only P0 login** (F-A03); (2) **Cloudflare Access** JWT validation (F-A02, P1); (3) **OIDC / Zitadel** (F-A01, P1, "high priority"). Exception: if Access *simplifies the infrastructure* needed for P0 it may land with or before user+password (**Q-0050.5**).
* **Consequence change:** the "no local break-glass login" bad-consequence above no longer applies; user+password is the baseline and remains as break-glass after OIDC lands. Password storage/reset/brute-force controls are now P0 scope (BetterAuth supplies them if the spike passes).
* **Not in sequence (untiered):** magic code (F-A04), Google/GitHub/GitLab/Gitea OAuth (F-A05) — Q-0050.3.
* **Authorization awareness is P0** (permission seam + RBAC, ADR-0051); **invites are P1**.
* API keys stay P0 (F-A06) for `/api/v1` (ADR-0031/0052).

### Consequences

* Good: smaller attack surface; SSO-only posture; consistent with existing Zitadel deployment; identity is a swappable port.
* Bad: dependence on Zitadel and Access availability once those phases land. (Superseded by Amendment A1 and round 25: user+password is the P0 baseline and remains as break-glass; machine clients reach `/api` through a path-scoped Access bypass, Q-0050.2.)
* Neutral: the first-admin bootstrap (first-boot `InstanceAdmin` sign-up in CE **[S]**) is replaced by a config-seeded admin (env/IaC); the admin app is retired (ADR-0021).

### Confirmation

Integration test: unauthenticated request → 302 to IdP; callback with unverified email rejected; tampered/foreign-audience Access JWT rejected on CF adapter; API key works without OIDC; contract test of the `Identity` port with a fake IdP.

## Pros and Cons of the Options

### Option 1: OIDC + Access + API keys

* Good: fits environment; minimal credentials to manage.
* Bad: break-glass and offline/self-hosted scenarios need an explicit story.

### Option 2: Full Plane parity

* Good: no surprise for existing users; works without Zitadel.
* Bad: password storage, reset and magic-code flows (Redis OTP + brute-force caps) to rebuild and secure; larger surface for little value in this deployment.

### Option 3: Access-only

* Good: simplest; no in-app auth code.
* Bad: no portability to the self-hosted twin; machine clients and public `spaces` need separate paths; no stable user identity/claims mapping unless parsed from JWT; tight coupling to one vendor.

## Update — interview round 13 (2026-10-17)

Spike bar (Q-0050.4): BetterAuth must run on Workers and Node, support user+password, use KV/Redis secondary storage for sessions, map to the wire-compatible `X-Api-Key`, and coexist with the `Authorizer` seam. If it fails, fall back to a hand-rolled `Identity` adapter behind the same port. (Superseded in round 29, FR-16: the bar is spike 0003's exit criteria. See the round-29 update.)

## Update — interview round 20 (2026-10-18)

Access scope (Q-0050.2, Q-0050.5): **Cloudflare Access gates UI hosts only; the API authenticates directly via session or `X-Api-Key`.** Access JWT validation joins at the CF Access phase, not P0. Clients: web apps only; third-party consumers are scripts/Beads via `/api/v1`; no mobile/desktop clients (Q-G8).

## Update — interview round 21 (2026-10-18)

BetterAuth is a TypeScript library; with a Rust backend the options are `better-auth-rs` (community alpha, wire-compatible with TS better-auth 1.7.x), TS BetterAuth as a service-bound identity Worker, or the hand-rolled Rust `Identity` fallback. See spike 0003 and `research/0002-rust-dioxus-fit.md` §2. Dioxus clients use cookies on web and bearer tokens on desktop/mobile.

## Update — interview round 23 (2026-10-18)

JIT provisioning via OIDC claims at P1 (Q-0050.1).

## Update — review decisions, round 25 (2026-10-18)

Spike 0003 bar amended: sessions are **DB-backed**, not KV/Redis. Import-era text about mapping imported data to the `Account` shape and the 'no local break-glass login' consequence is superseded by Amendment A1 (user+password at P0).
Access scope (round 25): Cloudflare Access is not used at P0 (user+password). At the CF Access phase it is applied with **path-scoped Access applications: UI paths gated; `/api`, `/auth` and `/live` bypass Access** because the application authenticates them itself (session, `X-Api-Key`, signed collab token). One same-origin hostname per product surface (ADR-0011); Access + WebSocket upgrade is verified in spike 0004.

## Update — review fixes, round 25 (2026-10-18)

Removed/annotated import-era text (Account mapping for imported data), the stale 'no break-glass login' consequence, the `god-mode` bootstrap, and marked Q-0050.1/.2/.5 answered. Decisions are unchanged.

## Update — spike results, round 26 (2026-10-18)

Spike 0004, public half (`research/0006-topology-public-half.md`): Cloudflare documentation, local workerd runs and one real-edge run **without Access applications**. The run with Access completed later the same day: the round-27 update below supersedes the `[doc]` and unverified tags in this note where it says so. The Zero Trust team is `woven-solutions`.

* **The line above that the API Worker "additionally verifies the Access JWT" must be read as optional on `/api`, `/auth` and `/live`.** Those paths are Access *bypass* applications (round 25), so their requests carry no valid Access JWT, and the Worker must **never trust `Cf-Access-*` headers on them**. Verify the JWT (signature against `https://<team>.cloudflareaccess.com/cdn-cgi/access/certs`, `aud`, `iss`) only on the gated UI paths; validating the header alone is not sufficient `[doc]`. On the real edge with no Access app, a forged `Cf-Access-Jwt-Assertion` header **reached** the Worker while a forged `Cf-Access-Authenticated-User-Email` did not (locally both arrived) `[tested-run1]`; re-measured in run 2 with Access on: the same result.
* **Path scoping `[doc]`:** an application is a hostname plus an optional path; `/api/*` does **not** cover `/api` itself, so list both forms for each of `/api`, `/auth` and `/live`; the more specific application wins and nothing is inherited from the parent; Bypass and Service Auth policies are evaluated before Allow; an unmatched request is blocked. **Was undocumented, measured in run 2 (round-27 update below):** `/api/*` covers deep paths; dot-segments and `%2e` are normalised before matching; a trailing-dot Host and an SNI/Host mismatch do not bypass.
* **Bypass requests are not logged by Access `[doc]`**, so `/api`, `/auth` and `/live` have no Access audit trail; the application audit log (ADR-0048) has to cover them.
* **`workers.dev` and preview URLs bypass hostname-based applications `[doc]`:** set `workers_dev = false` and `preview_urls = false` on every Worker. Run 1 confirmed that `workers.dev` returned 404 for the edge Worker `[tested-run1]`.
* **Worker-level Access answers WebSocket upgrades with 403 `[doc]`;** only hostname-based applications work. **Strict service-token mode** (the default for Zero Trust organisations created on or after 2026-10-05) answers failures with 401/403 rather than 302, and only Service Auth policies authorise requests that carry service-token headers `[doc]`.

## Update — owner decisions and spike results, round 27 (2026-10-18)

**Spike 0004 run 2 (real platform, Access gate on; `research/0006-topology-public-half.md` sections 13 and 14).** 35 assertions in three passes, each with a paired must-fail control; none void. Findings:

* `/api/*` **does cover deep paths** (`/api/v1/work-items/123/comments?x=1`, `/api/a/b/c`); `host/api` covers `/api/`; prefix lookalikes (`/apix`, `/authz`, `/liveness`) stay gated.
* The edge **normalises `..` and `%2e`/`%2E` before Access matches**, so a bypass prefix cannot be used to climb into a gated route (59 refusals). `..;`, `..%2f`, `%5c` and `%252e` are *not* treated as traversal: they stay under the bypass prefix and reach the Worker as `/api/…` paths. **The Worker must treat paths literally and must not canonicalise those forms into other routes.** `%00` and `..` above the root are rejected with 400.
* **The bypass match is case-insensitive (security-relevant).** `/Api/…`, `/API/…`, `/AUTH/…` and `/LIVE/…` skip Access, but the Worker's routing is case-sensitive, so with the SPA fallback they were answered with the 1,712-byte gated root shell and no credentials (7 forms; no secret marker appeared). Case variants of *gated* paths stay gated, and a nested gated app under `/api/gated/*` also matched case-insensitively and won by specificity. Rule: nothing reachable through a case variant of `/api`, `/auth` or `/live` may be non-public; either reject non-canonical-case paths in the Worker before any static fallback or run the Worker first (`run_worker_first: true`, billed per asset request). Decision: Q-0050.7.
* **Headers on bypass paths:** a forged `Cf-Access-Jwt-Assertion` reaches the Worker; a forged `Cf-Access-Authenticated-User-Email` is removed; `CF-Access-Client-Id` and `CF-Access-Client-Secret` are **passed through to the origin** (a client that attaches its service token to `/api`, `/auth` or `/live` delivers the secret to the API Worker and to anything that logs request headers there; scrub them, ADR-0130). Never trust `Cf-Access-*` on those paths.
* Gated paths hand the Worker a valid Access JWT, verified against the real team JWKS (RS256, `kid`, `exp`, `iss`, `aud`). An unauthenticated request to the UI application gets a 302 to login; to a service-auth-only nested application a **403**; OPTIONS on a gated path is 403.
* WebSockets work on the bypass path and on a gated path with a service token (text, binary, 1 MiB), and across a service-binding hop. A TLS SNI/Host mismatch between the two hostnames answers 403 (no domain fronting); a trailing-dot or upper-case Host and `:443`/`:8443` do not bypass.

## Update — review round 29 (2026-10-19)

Per FR-16 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now), "BetterAuth-shaped" means BetterAuth's HTTP shapes only, and the implementation is the Rust auth slice: spike 0003 as rewritten (`spikes/0003-auth-slice-rust.md`), built as issue B4. This supersedes Amendment A1's "Framework: BetterAuth" bullet and the round-21 note's three options. The end state, the phasing and the Decision are unchanged. BetterAuth is a TypeScript library, and its Rust port `better-auth-rs` is a community alpha whose wasm32 support is undocumented (`research/0002` §2). Spike 0003 therefore drops its options A (`better-auth-rs`) and B (a TypeScript identity Worker), and C, hand-rolled Rust, is the plan.

* **The round-13 spike bar is superseded.** It still asks for "KV/Redis secondary storage for sessions", which contradicts ADR-0052 round 25 (the database row is authoritative, no cache at P0) and FR-03 (sessions are rows behind the `Database` port; ADR-0052, round 29). Round 25 amended the bar for sessions; this replaces the rest of it. The bar is now spike 0003's exit criteria: argon2id with OWASP parameters, its CPU time measured on a deployed Worker and inside the default 30 s limit with margin; a session row with an HttpOnly cookie and a CSRF double-submit token; hashed API keys accepted as `X-Api-Key`; and one OIDC authorization-code login through the `openidconnect` crate against Zitadel or a mock IdP. All of it runs on both targets, through the user-only `Principal` (FR-03; ADR-0051, round 29), and passes the contract suite on its four targets.
* **Q-0050.4 is answered:** a hand-rolled Rust implementation; spike 0003 as rewritten is its evidence and has not run yet. Per FR-05 (same review and §5 table), identity is core code over the `Database` port, not a port of its own (ARCHITECTURE §1c, round 29); this ADR's `Identity` port and A1's `Identity`/`SessionStore` ports are read that way. The known unknown is the OIDC client on wasm32: its HTTP calls need a client built on `worker::Fetch`. The slice's tests cover part of the Confirmation above: a callback with an unverified email is rejected, an API key works without OIDC, and the login is tested against a fake IdP.
* **Q-0050.6 is answered:** nothing is imported (Q-G3), and the `User`, `Profile` and `Account` tables are defined fresh in the schema; nothing is mapped onto a BetterAuth schema. The Decision's `User` + `Account(provider, provider_account_id)` shape stands, and "the final schema may follow the chosen identity implementation" now means the tables the Rust implementation defines.
* A1's "BetterAuth supplies them if the spike passes" (password storage, reset and brute-force controls) is superseded: the Rust implementation supplies them. The slice covers argon2id hashing and reviews the sign-in rate limit (ADR-0033, round 29); password reset is not in it.

## Open Questions

* Q-0050.1 — answered (round 23): JIT provisioning via OIDC claims at P1.
* Q-0050.2 — answered (rounds 20, 25): the API bypasses Access (path-scoped); machine clients use `X-Api-Key`.
* Q-0050.3 — Keep email/password, magic code, other OAuth? Tier each. (partly answered, A1)
* Q-0050.4 — BetterAuth fit behind our ports (spike). (answered round 29, FR-16)
* Q-0050.5 — answered (round 20): Access JWT validation joins at the CF Access phase, not P0.
* Q-0050.6 — Plane `User`/`Account`/`Profile` ↔ BetterAuth schema mapping. (answered round 29, FR-16)
* Q-0052 — sessions and API-key mechanics.

## More Information

[[ADR-0002]], [[ADR-0031]], [[ADR-0051]], [[ADR-0052]], [[ADR-0053]]. Plane: `apps/api/plane/authentication/` (adapter/base.py `complete_login_or_signup`); Open-EE: `woven/plan:.claude/plans/plane-fork/features/oidc-saml-sso.md`, `research/auth-permissions.md`. Cloudflare Access JWT validation docs (verify when implementing).
