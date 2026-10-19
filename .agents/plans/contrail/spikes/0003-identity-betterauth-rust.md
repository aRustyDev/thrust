# Spike 0003 — Identity: BetterAuth with a Rust backend (and the Dioxus client)

**Gates:** ADR-0050 (+A1), ADR-0052, ADR-0053, ADR-0051 (seam coexistence).
**Questions:** Q-0050.4, Q-0050.6, Q-0050.1, Q-0052, Q-0033.
**Timebox:** 4 working days. **Depends on:** spike 0001 (Worker target); uses spike 0002's candidate engine (Postgres is enough to start). **Owner:** TBD.

## Question
How do we get BetterAuth-quality identity (user+password at P0 → CF Access → OIDC/Zitadel; sessions; API keys) when the
backend is Rust? BetterAuth is a TypeScript library.

## Options to evaluate
- **A. `better-auth-rs`** (community Rust port, alpha, targets wire-compat with TS better-auth 1.7.x; SeaORM adapter;
  email/password, sessions, API key, generic OIDC, organizations). Unknown: `wasm32`/Workers support.
- **B. TypeScript BetterAuth as a separate "identity" Worker** (service-bound) that the Rust API trusts via session
  lookup or signed token. Keeps the real library; adds a second language/runtime and a second deployable.
- **C. Hand-rolled Rust `Identity` adapter** behind the port (argon2, sessions in KV/DB, API keys hashed, OIDC via an
  audited crate), borrowing BetterAuth's schema and HTTP shapes.

## Method
1. For each option, implement: sign-up/sign-in with password, session cookie, sign-out, **sign-out-everywhere**,
   30-day session (ADR-0052), API-key create/verify hashed and accepted as `X-Api-Key` (ADR-0033).
2. Run on Workers (workerd) **and** native; **sessions are database-backed with no cache** (round 25), identical on both targets.
3. OIDC login against a Zitadel instance (or mock) — needed at P1, prove feasibility only.
4. Coexistence with the `Authorizer` seam: principal → capabilities; no auth logic in handlers.
5. Schema fit: map Plane's `User`/`Account`/`Profile` onto the option's tables (Q-0050.6).
6. **Clients:** existing React app (TS better-auth client, cookies) works; **Dioxus clients** — web (cookie, same-origin),
   desktop/mobile (no browser cookie jar semantics → bearer-token plugin or token flow). Prove a Rust HTTP client
   (reqwest/gloo-net) can complete sign-in and make an authenticated call.
7. Security review checklist: password hashing params, session fixation, CSRF, rate limiting of auth endpoints, cookie flags.

## Exit criteria
Option passes if all of 1–5 work on both targets and a Rust client completes 6. Prefer A or B only if their maintenance
and security posture is acceptable (A is alpha — require pinned version + owned fork plan); otherwise C.
Fallback is already recorded in ADR-0050: hand-rolled `Identity` adapter behind the same port.

## Deliverables
Decision in ADR-0050/0052; client-auth notes for ADR-0013; Q-0050.4/.6 closed.
