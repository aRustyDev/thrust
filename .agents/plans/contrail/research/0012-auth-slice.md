# Research 0012 — The auth slice: spike 0003's results

Date: 2026-10-23 · Spike `spikes/0003-auth-slice-rust.md` · Issue [#38](https://github.com/aRustyDev/thrust/issues/38) (FR-16) · Bead `plane-byd.11.5` (children `.1` to `.9`) · Branch `feat/auth-slice` · ADR-0050, ADR-0052 (Q-0050.4 and Q-0050.6 are answered in round 29; this is the evidence behind the first)
Status: **done. Verdict: option C, hand-rolled Rust, delivers the P0 credentials and one OIDC login, with the same behaviour on the wasm32 Worker and the native server. K1 and K4 pass; K3 is recorded, not decided; K2 was not exercised. The Zitadel login and the OIDC path from a deployed Worker were not run.**

Evidence tags: **[re-run]** I ran it in this work, with the command given · **[tested-cf]** run against a Worker deployed on Cloudflare's network (the Woven account) · **[derived]** read from source or docs, not run · **[unverified]** not established (section 9).

## 0. Read this first

1. **The four exit criteria hold, with one of them narrowed.** The contract suite's main pass is green on all four targets with the count raised from 21 to **37 tests**; argon2id fits the platform by a factor of 300 or more; the same statuses, cookie attributes and refusals come out of the Worker targets as out of the native ones; and one OIDC login completes through `openidconnect` on all four targets against a mock provider. Against **Zitadel** it was not run (section 9), so exit criterion 4's "and once against Zitadel (K2)" is open.
2. **K1 (`openidconnect` on wasm32): passes.** It builds for `wasm32-unknown-unknown` with `default-features = false`, a client built on the `Fetch` port (reqwest natively, `worker::Fetch` on the Worker) gives futures that meet the `Send` rule, and the whole login, including the RSA signature check of the ID token, runs as wasm under workerd. It cannot live in `thrust-core`, because it pulls `wasm-bindgen` and `js-sys` into the wasm32 graph; it is its own crate, `thrust-adapters-oidc` (section 3).
3. **K4 (argon2id CPU on a deployed Worker): passes.** At OWASP's minimum (19 MiB, t = 2, p = 1) one sign-in costs **45 ms CPU at p50 and 78 ms at p99** on the platform; the worst of the five equivalent configurations has a p99 of 93 ms. The default limit is 30 s. Natively, the same build does the minimum in 12.6 ms. **[tested-cf]**
4. **K3 (CSRF against the stock React app): the gap is real and is recorded** (section 5): a cookie-authenticated mutation without the token is refused, and the stock app sends no token on its JSON calls. The slice does not decide what M1 does about it.
5. **ADR-0052's line 53 said a revoked key is 401. It is 403.** The oracle answers 403 for an invalid, expired, revoked or inactive-owner key and 401 only for a missing one (ADR-0033, round 27). The contract tests assert it and ADR-0052 is corrected.
6. **Five design choices that the plan left open or got wrong** are in section 2: the OIDC adapter is its own crate, a session of a user that does not exist is not live, a verified email is always required (also for a linked subject), API keys are hashed without a pepper, and the CSRF header is `X-CSRFToken`.

## 1. What was built

| Piece | Where | What it is |
|---|---|---|
| Tables | `thrust-adapters-postgres/migrations/0005_identity.sql` | `users`, `accounts(provider, provider_account_id)`, `api_keys` (SHA-256 digest, nine-character prefix, expiry, revocation, last use) and `oidc_logins` (digest of the `state`, nonce, PKCE verifier, expiry). Directory tables (ADR-0041); no column holds a plaintext token, key or password |
| Ports | `thrust-ports` | `Rng`, `PasswordHasher`, `Fetch`, `OidcProvider`, and identity methods on `Database` (users, accounts, API keys, pending logins, sign-out-everywhere) |
| Core | `thrust-core/src/auth.rs` | `sign_in`, `mint_session`, `sign_out_everywhere`, `authenticate_request` (which credential counts, when a mutation needs the CSRF token), `csrf_ok`, the API-key use-cases, `oidc_start`, `oidc_callback`, `Argon2idHasher` |
| OIDC | `thrust-adapters-oidc` | `OpenIdProvider`: the `openidconnect` crate over the `Fetch` port |
| Adapters | memory, postgres, selfhosted, cloudflare | The identity methods in memory and Postgres; `NativeRng`, `ReqwestFetch`; `JsRng`, `WorkerFetch` |
| HTTP | `thrust-api` | Ten routes under `/api/_slice` (sign-in, sign-out, sign-out-all, csrf, session, the three API-key routes, the two OIDC routes), the `Authed` extractor on the three credentials |
| Suite | `thrust-contract-tests` | Six identity tests and a generated matrix (`tests/auth/`), a mock identity provider (`src/mock_idp.rs`), and the main-pass count at 37 |
| Controls | `scripts/mutants.sh` | `skip-csrf` and `unverified-email` rows, and `ignore-revoked` now also breaks API keys |

The routes, statuses and flow are on [Application flow](../../../../docs/src/flows/app.md); the crates on [Application architecture](../../../../docs/src/architecture/app.md).

## 2. Design choices, and where they differ from the plan

| Choice | Why | Differs from the plan? |
|---|---|---|
| **`openidconnect` lives in `thrust-adapters-oidc`, behind an `OidcProvider` port, not in core** | On wasm32 it pulls `chrono`'s `wasmbind` and `getrandom`'s `js` backend, so `wasm-bindgen` and `js-sys` enter the graph. `scripts/check-deps.sh` forbids those in `thrust-core` and fails when core takes the dependency (shown, section 8). The port also lets the core tests script a provider | The plan said core would import `openidconnect` and only the HTTP client would be an adapter. Same decision (ADR-0050), different seam; no ADR text changes |
| **A session is live only for an existing, active user** (`sessions` is joined to `users`) | Fail closed: a deactivated user's sessions and keys stop at once (ADR-0052 Tests), and a session of a user with no row is not a session. No foreign key, so a database with old rows needs no backfill | The plan said "honours `users.is_active`"; it did not say what a missing row is. The memory session tests now create the session's user (assertions unchanged) |
| **A verified email is required on every OIDC login, also for a subject that is already linked** | ADR-0050 says "verified email required" and the guard against account takeover (GHSA-7j95-vh8g-f365) is the point of it; checking it only on the email path would let an IdP that stopped verifying emails keep logging in linked users | Stricter than the plan's "else a user with the same **verified** email" |
| **API keys are SHA-256, no pepper** | The token is 256 random bits, so there is nothing to guess and nothing to stretch; the same reasoning as the owner's 2026-10-22 decision for sessions. ADR-0141's "peppered hashes" are therefore not applied to keys: recorded as Q-0052.3 | The issue says SHA-256; ADR-0141 says peppered |
| **CSRF header `X-CSRFToken`, cookie `__Host-thrust_csrf`, token from `GET /auth/csrf` as JSON** | Django's and Plane's header name and Plane's `get-csrf-token` shape, so the stock auth client (which already calls that route) needs the least change (section 5) | None |
| **Cookies `Secure; HttpOnly; SameSite=Lax; Path=/`, no `Domain`, `__Host-` prefix, `Max-Age` 30 days** | `__Host-` stops a sibling subdomain planting a cookie; `HttpOnly` also for the CSRF cookie, as Plane does; `Lax` and not `Strict`, because the OIDC callback is a top-level navigation whose cookie must reach the next page | The issue says "`SameSite`" without a value |
| **The `state`, nonce and PKCE verifier are core's, in a database row; the adapter holds nothing per login** | A Worker isolate can die between the two legs; the row is single-use (taken and deleted in one statement), so a replayed callback finds nothing, and it is checked before the provider is asked anything | The plan named the table; this is its use |
| **Every OIDC call discovers the provider again** (a metadata and a key-set fetch) | No cache to go stale or to differ between targets; two extra round trips per leg is not a P0 problem | A cache is a later optimisation |
| **Keys and sign-out take a session, not a key** | A leaked key must not be able to end the owner's sessions or mint more keys | Not in the plan |
| **Test targets hash at m = 64 KiB, t = 1** (`scripts/contract.sh`) | The suite signs in many times; OWASP's cost is pinned by a core test and measured on a deployed Worker | The plan said "cheap parameters through config" |
| **Sign-in limiter: 10 a minute per normalised email by default, before any hashing** | Spike step 6; keyed by email, so a respelled email shares the count. A burst of wrong guesses can lock an account for the window: accepted at P0, recorded in section 6 | The limiter is tested in core, not over HTTP (a fixed window that a burst can straddle needs the create test's bespoke retry) |
| **Defect ids X21 to X24** | The matrix already ran to X20 (and has X15 twice), so "the next four after X15" was taken. The duplicate X15 is untouched | The plan's numbering was out of date |
| **The deployed measurement used the memory adapters, not Hyperdrive** | argon2's CPU does not depend on the adapter set, and the memory set needs no resource but the Worker | The spike text said to share `worker-pg`'s deployment |

## 3. K1: `openidconnect` on wasm32

Probe, in a scratch crate outside the repository (bead `.1`): `argon2` 0.6.0 (password-hash 0.6.1), `openidconnect` 4.0.1 with `default-features = false` (`oauth2` 5.0.0, `chrono` 0.4.45, `rsa` 0.9.10, `p256` and `p384` 0.13), `worker` 0.8.7, `getrandom` 0.4.3 with `wasm_js`.

- **It builds** for `wasm32-unknown-unknown` with no cfg flags. **[re-run]** `scripts/rs cargo check --target wasm32-unknown-unknown`.
- **Its futures are `Send`.** A client implementing `oauth2::AsyncHttpClient` over `worker::Fetch`, with `Future = Pin<Box<dyn Future + Send>>` built on `worker::send::SendFuture`, compiles, and a function awaiting `discover_async` and `exchange_code` passes an `is_send` check. **Control:** without `SendFuture` it fails with E0277 (`Rc<RefCell<futures::Inner>>` cannot be sent between threads). **[re-run]**
- **What it costs in the graph.** `tokio` and `hyper` are not pulled (`tokio` reaches the Worker only through `worker`). `wasm-bindgen` and `js-sys` are, through `chrono` and `getrandom` 0.2; hence the separate crate and a section in `check-deps.sh`: the crate may have those two on wasm32 and nothing else forbidden, and `thrust-core` must not depend on it. **Controls** [re-run]: giving the crate `openidconnect`'s default features fails the check (tokio, mio, socket2, hyper, hyper-util), and giving `thrust-core` the dependency fails it (wasm-bindgen, js-sys in core and in the API crate).
- **It runs on workerd.** The six `oidc_` contract tests pass on `worker-memory` and `worker-cf` (wrangler dev, local workerd): discovery, the token exchange with PKCE, JWKS and **RS256 verification** all execute as wasm, and the Worker's subrequest reaches the mock on `127.0.0.1`. **[re-run]**
- **The provider is checked independently.** The mock (`crates/thrust-contract-tests/src/mock_idp.rs`) writes its discovery document by hand and signs with the `rsa` crate, so `openidconnect` in the target and the signer share no code. It checks, on the browser's leg, that the authorization request carries `response_type=code`, the right client and redirect URI, `openid` and `email` scopes, a 256-bit `state` and `nonce`, and an S256 PKCE challenge; and on the token endpoint, the client secret, the redirect URI and the PKCE verifier. A control in the login test shows the mock refuses a wrong verifier, so the logins that pass are not passing against a mock that accepts anything.
- **ID-token verification is exercised, not assumed.** A wrong `nonce`, another audience, another issuer, an expired token and a corrupted signature are each refused (401, no cookie) on all four targets. **[re-run]**

Size: the Cloudflare set (`worker-build --release . --no-default-features`) is **2,505 KiB raw (2.45 MiB), 822 KiB gzip** [re-run, `wc -c` and `gzip -9`]; the deploy of the build with the memory set too uploaded 2,680 KiB / 874 KiB gzip, Worker startup 2 to 3 ms **[tested-cf]**. Spike 0001's Stage 2 bundle was 1.67 MiB / 568 KiB; the part of the growth that is auth and OIDC was not separated out (no before/after on `origin/main` was built).

## 4. K4: argon2id CPU on a deployed Worker

**Method.** `scripts/measure-argon2.sh` deploys a throwaway memory-adapter Worker (`thrust-spike-auth`, `crates/thrust-worker/wrangler.measure.toml`) once per OWASP configuration, with the cost passed as `THRUST_ARGON2_*` vars; seeds a user; signs in 200 times over one connection (seeding again every 20); and reads each invocation's `cpuTime` and `wallTime` from a tail consumer that stores them in KV (`wrangler tail` does not print `cpuTime`, and a Tail Worker's own console output is not streamed). 1,016 of 1,016 invocations were matched to their client request. Account: Woven, Workers Paid (`default_usage_model: standard`). **[tested-cf]**

| Configuration (p = 1) | sign-ins | found the user (200) | CPU p50 | CPU p99 | CPU max | native p50 |
|---|---|---|---|---|---|---|
| m = 46 MiB, t = 1 | 200 | 200 | 55 ms | 93 ms | 96 ms | 15.6 ms |
| **m = 19 MiB, t = 2 (OWASP's minimum)** | 200 | 186 | **45 ms** | **78 ms** | 85 ms | 12.6 ms |
| m = 12 MiB, t = 3 | 200 | 200 | 46 ms | 89 ms | 97 ms | 11.8 ms |
| m = 9 MiB, t = 4 | 200 | 200 | 41 ms | 74 ms | 93 ms | 11.8 ms |
| m = 7 MiB, t = 5 | 200 | 200 | 40 ms | 78 ms | 80 ms | 11.8 ms |
| minimum, 16 at once on separate connections | 16 | 1 | 90 ms | 194 ms | 194 ms | |

- **The margin.** The worst p99 of the five is 93 ms against a default CPU limit of 30,000 ms: **a factor of about 320**; the worst single invocation, 194 ms (the burst), about 150 times. No invocation had an outcome other than `ok`: none exceeded the CPU or the memory limit. **[tested-cf]**
- **Wasm against native.** The platform's p50 is 3.4 to 3.9 times the native one for the same build profile (`opt-level = "s"`, the size profile of the Worker bundle; `cargo run --release -p thrust-core --example argon2_timing`, 100 runs each, this Apple-silicon machine). A speed-tuned profile for `argon2` and `blake2` was not tried; it is not needed. The five configurations cost about the same on the platform (40 to 55 ms), because the fewer passes over more memory trade evenly against more passes over less. **[re-run]**
- **Found the user or not.** The `200` column is how many sign-ins found their user. The memory adapter's state is per isolate, and a request does not always reach the isolate that was seeded (the burst: 1 of 16). A refusal costs the same single hash by design, and the core test counts it (`sign_in_succeeds_exactly_for_an_active_user_with_the_right_password_and_costs_one_hash_either_way`); in this run the p50 of the 401s and of the 200s at the minimum are 44 and 46 ms.
- **Memory.** The platform reports no memory figure, so none is claimed. The largest configuration allocates 46 MiB of wasm linear memory (which an isolate keeps once grown) and no invocation exceeded the 128 MiB limit. Hashing is synchronous with no await inside, so concurrent sign-ins in one isolate run one after the other and their memory does not add up; the cost of concurrency is latency instead (the burst's wall p50 was 179 ms against a CPU p50 of 90 ms), and one isolate does about 22 sign-ins a second at the minimum. **[derived]** from the code and the burst.
- **Cold start.** The first request after a deploy cost 58 ms at the first configuration and between 42 and 85 ms elsewhere: the wasm instantiation is inside it and is small.

**Projected cost** (envelope < $100 a month; Workers prices as in `research/0004` §9, 30 M CPU-ms included then $0.02 per million **[derived]**): at the sizing target, 50 users signing in about once a day is about 1,100 sign-ins a month, 1,100 × 45 ms ≈ 50 s of CPU, which is under 0.2 % of the included allowance: **about $0**. Even 20 sign-ins per user per day, 22,000 a month at 45 ms ≈ 1 M CPU-ms, stays inside it.

**Native.** `argon2_timing` prints, per configuration, p50, p99, max and mean of 100 hashes; the table's native column is its p50.

## 5. K3: CSRF against the stock React app

ADR-0052 asks for the token on every cookie-authenticated mutation. Plane disables its CSRF check on its REST API (`apps/api/plane/authentication/session.py:10`), and the React app sends the token only on its auth form posts and on `set-password` (`packages/services/src/auth/auth.service.ts:79,122`), which it fetches as JSON from `GET /auth/get-csrf-token/` (`auth.service.ts:34`). So the stock app's JSON mutations, authenticated by the session cookie, would get **403 `csrf_failed`** from this backend. The contract test `a_cookie_authenticated_mutation_needs_the_csrf_token` pins the refusal for POST and DELETE; the generated matrix pins it for every credential state. **[re-run]** The React app itself was not run: this is read from the cited source.

The slice records the gap and does not decide it. What M1's auth-client change (a Q-R2 change area, `research/0003` §9) can do, from least to most change: send `X-CSRFToken` from the token the app already fetches, on every JSON mutation (one interceptor in the service layer); or let the cookie's `SameSite=Lax` plus an `Origin` check stand in for the token on the JSON API, which would be a new ADR-0052 decision. Open as Q-0050.8.

## 6. Security checklist (spike step 6)

| Item | State |
|---|---|
| argon2id parameters | OWASP's minimum by default (m = 19,456 KiB, t = 2, p = 1), pinned into the encoded hash by a core test (`$argon2id$v=19$m=19456,t=2,p=1$`); configurable; a hash carries its own cost and verifies under any configured one, so raising the default locks nobody out. Salt: 16 bytes from `Rng` |
| Password bounds | 1 to 1,024 bytes; email up to 320 characters; refused with 400 before anything is hashed, for every caller alike |
| Session fixation | Every sign-in and every OIDC login mints a new token; a session id is never accepted from the client |
| Token entropy | session 256 bits, CSRF 128, API key 256 (`thrk_` + 64 hex), OIDC `state`, nonce and verifier 256 each; hex from the `Rng` port; a failing source is a 500, never a short token |
| Storage | Only SHA-256 digests of session tokens, API keys and the OIDC `state`; argon2id PHC strings for passwords; the nonce and verifier are stored readable for at most ten minutes and deleted when taken. `pg_adapter` pins the column lists |
| Cookies | `__Host-thrust_session` and `__Host-thrust_csrf`; `Secure; HttpOnly; SameSite=Lax; Path=/`; no `Domain`; `Max-Age=2592000`; cleared (empty, `Max-Age=0`) on sign-out; every response that sets or clears one is `Cache-Control: no-store`. The suite asserts the attributes on `Set-Cookie` itself and sends the cookies back by hand, because the local targets serve plain HTTP (a cookie jar would withhold a `Secure` cookie) |
| CSRF | Double submit: `X-CSRFToken` must equal the CSRF cookie (both present, non-empty), compared in constant time (`subtle`); a bearer token and a key are exempt; safe methods are exempt; a dead session is 401 before the token is looked at |
| Timing | A wrong password, an unknown email, a user with no password: one unit of hash work each (counted, not timed), one 401, one body. A deactivated user is the same 401 |
| Rate limiting | Per normalised email, before hashing, through the `RateLimiter` port: 10 a minute by default, 429 with `Retry-After`. Core-tested; the contract targets use 30 so no test comes near it. A burst can lock one account for the window (denial of service on a single user): accepted at P0 |
| API keys | Shown once; revoke is effective on the next request; keys cannot mint keys or end sessions; 403 for every invalid state, 401 only for a missing or empty header; `last_used_at` is written through `Background`, at most once an hour |
| OIDC | PKCE S256; `state` single use, ten minutes, checked before the provider is asked; nonce and audience, issuer, expiry and signature verified by `openidconnect`; verified email required; link by `(issuer, sub)` then by email; no provisioning; deactivated users refused |

**Not done, on purpose or for later:** password reset, sign-up, email verification of local accounts (a local user is trusted as seeded), a password-strength or breached-password check, an idle session timeout, rotating a session id on privilege change, re-hashing at a new cost on sign-in, account lockout beyond the limiter, SCIM (ADR-0053), `Cf-Access-*` JWT verification.

## 7. Exit criteria

| # | Criterion | Result |
|---|---|---|
| 1 | The contract suite passes on four targets, count raised, every step-7 control failing the suite | **Met.** Main pass 37 of 37 on `native-memory`, `native-pg`, `worker-memory`, `worker-cf` (`scripts/contract-all.sh` 16 of 16); controls in section 8 |
| 2 | argon2id fits the platform: p99 CPU on the deployed Worker fits 30 s with margin | **Met.** 78 ms at OWASP's minimum, 93 ms the worst of five **[tested-cf]** |
| 3 | Same behaviour on both targets: status codes, cookie attributes, refusals | **Met** for the local Worker targets (workerd) and the native ones: the same assertions pass on all four. The deployed edge was not exercised (section 9) |
| 4 | One OIDC login, through `openidconnect`, on both targets against the mock, and once against Zitadel | **Half met.** The mock, on all four targets. Zitadel: **not run** |

## 8. Controls (AGENTS.md rule 8)

Every control was run and failed the check it is for; counts are the tests that failed. **[re-run]**

| Check | Broken variant | Failed |
|---|---|---|
| Hermetic suite | 576 tests pass in `cargo nextest run --workspace`; the 63, 123, 50 and 51 new core cases (`auth`, `credentials`, `api_keys`, `oidc`) were each red against stubs first (47 of 63, 100 of 123, 50 of 50, 51 of 51) | |
| Sign-in | no hash work for an unknown email · a distinguishable refusal for an inactive user · an 8-byte token · no limiter · a 4 MiB default · a `verify` that always accepts | 26 · 10 · 2 · 4 · 2 · 40 |
| CSRF and credentials | CSRF on a bearer token · on a `GET` · empty tokens accepted · length-only comparison · a bad bearer falling through to the cookie · `mutant_skip_csrf` | 20 · 10 · 4 · 8 · 24 · 54 |
| API keys (core) | a revoked key resolving · 401 for an invalid key · 403 for a missing one · a 128-bit token · a long prefix · a stamp on every request · revoke ignoring the owner | 6 · 24 · 2 · 2 · 4 · 2 · 2 |
| API keys (HTTP) | a revoked key accepted · expiry ignored · an inactive owner accepted · 401 for 403 · a key allowed to mint keys | 1 · 1 · 1 · 3 · 1 test |
| OIDC (core) | the provider asked before the state is checked · the caller's nonce · expiry ignored · an inactive user accepted · the email beating the link · a reusable state · `mutant_unverified_email` | 80 · 16 · 16 · 4 · 2 · 16 · 4 |
| OIDC (HTTP) | `mutant_unverified_email` · a wrong PKCE verifier · a wrong PKCE challenge · an unchecked nonce · a reusable state | 1 · 6 · 6 · 1 · 1 test |
| Cookie attributes (HTTP) | no `Secure` · no `HttpOnly` · `SameSite=Strict` · no `Max-Age` | 2 · 2 · 2 · 3 tests |
| Credential matrix (HTTP) | the CSRF token checked before the cookie is authenticated (**only the matrix fails**) · any non-empty token accepted · a bearer token that needs the token | 1 · 2 · 21 tests |
| Postgres adapter | a plaintext column · a missing digest check · and, against the generated model, nine broken queries (session ignoring `is_active`, a take that keeps the row, key expiry, revoke ignoring the owner, revoke-all across users, a lost `COALESCE`, a replaced link, no sweep, touch-once), each failed in every run measured (6 to 10 runs at the default 32 cases; the first versions of four survived most runs and the compound scenarios were added) | |
| Memory adapter | six broken adapters (session ignores user state, sign-out-everywhere across users, take keeps the row, expiry at the boundary, revoke ignores the owner, a link replaced) | 6 · 12 · 4 · 2 · 2 · 2 |
| Layering | `check-deps.sh` fails when the OIDC crate takes default features, and when core takes `openidconnect` | |
| `scripts/mutants.sh` | **7 of 7 KILLED, no `SocketNotFoundError`:** ignore-tenant 6 tests, duplicate-sequence 5, no-activity-dedupe 2, ignore-revoked 5, skip-csrf 5, unverified-email 1, postgres discard-noop 2 | |

The first versions of the contract tests for sign-in, keys and OIDC were written after their routes, so their red run is the table above, not an absent route; the commit messages say so.

## 9. What the timebox did not prove

- **The Zitadel login (K2).** No login against `id.auth.woven`: it is WARP-only, and the owner chose the mock provider only. Nothing here shows that Zitadel's discovery document, key set or ID tokens parse exactly as the mock's do; they are standard OIDC and `openidconnect` is a mainstream implementation, which is not evidence.
- **The OIDC path from a deployed Worker.** The mock is on `127.0.0.1`; a deployed Worker cannot reach it, so `scripts/stage2-contract.sh` skips the six `oidc_` tests (it expects 31 of 37), and nothing exercised `worker::Fetch` against a remote issuer. How a deployed Worker reaches a WARP-only issuer (a route to it, a Workers VPC service) is recorded for M6 (Q-0050.9).
- **The deployed contract pass.** `scripts/stage2-contract.sh main` was not run against a deployed Worker with Hyperdrive; the deployed run measured the memory set only. The cookie, CSRF and status behaviour was seen only through local workerd.
- **A browser.** No browser touched the cookies; attribute behaviour (`__Host-`, `Secure` over `localhost`, `SameSite=Lax` on the callback navigation) is read from the specifications, and the stock React app was not run (K3 is read from source).
- **Memory on the platform,** as section 4 says; and the contract suite's `blast` and `bench` passes with identity routes.
- **Session expiry on a real clock** (the core tests use a fixed one); **concurrent sign-ins against one user** and the limiter's per-window straddle over HTTP; **a password reset, sign-up or lockout flow**, which do not exist.
- **Planted mutants for the sign-in logic.** The sign-in controls were applied by hand and are listed in the commit messages; only `skip_csrf`, `unverified_email` and `ignore_revoked` are planted features.

## 10. Reproduce

```sh
scripts/rs cargo nextest run --workspace                 # 576 hermetic tests, including the identity ones
scripts/rs cargo test --doc --workspace
scripts/check-deps.sh
DOCKER_HOST=unix:///Users/asmith/.orbstack/run/docker.sock scripts/contract-all.sh   # 16 of 16, main passes at 37
DOCKER_HOST=unix:///Users/asmith/.orbstack/run/docker.sock scripts/mutants.sh        # 7 of 7 KILLED
scripts/rs cargo run --release -p thrust-core --example argon2_timing -- 100         # the native half of section 4
CLOUDFLARE_ACCOUNT_ID=<cloudflare-account-id> scripts/measure-argon2.sh 200          # the deployed half; needs wrangler OAuth
```

## 11. Resource ledger (the deployed run)

Everything was named `thrust-spike-` and deleted. **Created:** Workers `thrust-spike-auth` and `thrust-spike-auth-tail` (five runs of the script, each with five deploys of the first Worker; the run that is reported is the last), the Worker secret `THRUST_ADMIN_TOKEN` (part of the first Worker), KV namespaces `thrust-spike-auth-cpu` (one in each of the last four runs) and `thrust-spike-auth-cpu-probe` (one probe). **Confirmed gone**, after the last run: `wrangler deployments list --name` for both Workers reports no such Worker; `wrangler kv namespace list` shows only the namespace that was there before; the Workers API lists only the account's own two Workers (`art-filestore`, `workflows-starter-template`), which were not touched. **Credentials:** wrangler's OAuth login (scopes `workers`, `workers_kv`, `workers_tail`) for every write; the `thrust-spikes` account token only for read-only calls (list Workers, the workers.dev subdomain, account settings). Whether that token can write Workers is still unverified, and was not needed. Cost: about 2,800 requests to the Worker and 2,300 KV writes across all runs, well inside the Workers Paid allowances.
