# Research 0006 — Spike 0004, public half: Worker topology, path-scoped Access, WebSocket

Date: 2026-10-18 · Spike: `spikes/0004-topology-and-woven-tunnel.md` (bead `plane-byd.2`, wave 1) · Code: `scripts/spike-0004/`
Status: **public half RUN on the real platform (run 2, 2026-10-18, §13) and torn down (§14); `.woven` half remains a desk recipe (§7).**
History: §0-§9 are the pre-run write-up (originally tagged before any platform access). Run 1 (§10) deployed and created custom domains but the Access policy POST
returned `403 auth.forbidden` (token lacked "Access: Apps and Policies Edit"), so it was halted and torn down by the coordinator (§11). A first attempt at run 2 was blocked by a
1Password timeout before any Cloudflare call (§12). Run 2 (§13) ran start to finish with the gate ON: 35 assertions per pass, three passes, two latency passes, then teardown (§14).
Headline results: path-scoped Access (gated UI app + Bypass app for `/api`, `/auth`, `/live`) works as designed, WebSocket works through bypass, gated and service-binding paths,
**but the bypass match is case-insensitive (T1 FAIL: the gated UI shell is served unauthenticated at `/Api/…`)** and **a service-binding hop costs about 2.3-2.8 ms, not zero**.
Leftover: the two edge certificate packs (unreadable and undeletable with this token). §0-§9 keep their original tags where not superseded; §10-§14 use `[tested]` / `[tested-run1]`
(executed against the real Woven account); a few §0-§9 rows were updated in place and say so.

Evidence tags used throughout (every claim carries exactly one):

| Tag | Meaning |
|---|---|
| **[doc]** + URL | stated on a Cloudflare page I read this session (raw `.md` of the page, or the provider docs on GitHub) |
| **[measured-local]** | observed running the Worker under `wrangler dev --local` 4.123.0 (workerd, no Cloudflare). True of workerd; says nothing about Cloudflare's edge unless a doc says the two match |
| **[model]** | the harness's behaviour against a *mock gate* (a model of the documented rules). Proves the scripts discriminate; proves nothing about Cloudflare |
| **[derived]** | arithmetic or inference from tagged facts, assumptions stated |
| **[estate]** | from the woven rules (`woven-warp-hostnames.md`, `woven-status.md`), not from a Cloudflare page |
| **[unverified]** | not established; each one is a harness check or a runbook step |

## 0. Read this first

1. **The headline Access question is now answered on the real platform (run 2, §13), with one security finding.** Path-scoped Access
   with a gated UI app and a Bypass app for `/api`, `/auth`, `/live` works as designed in round 25: the bypass paths (exact, trailing-slash and
   deep forms) reach the Worker with no credentials, everything else is refused without a service token, a nested gated app wins by specificity,
   and WebSocket upgrades pass through both the bypass and the gated app. **But Access's bypass destination match is case-INSENSITIVE** (BF3/T1):
   `/Api/x`, `/API/ping`, `/AUTH/callback` are treated as bypass, and with the Worker's case-sensitive routing they fall through to the SPA fallback,
   so the gated UI shell is served with no credentials. No secret marker leaked; the traversal/encoding/Host/SNI probes found no other leak.
   §0 items 2-3 and §1-§9 below are the pre-run write-up and keep their original tags; the observed results are in §13.
2. **The harness was validated by discrimination, not by passing.** It was run against eight targets: two correct and six
   deliberately broken or mis-configured (§2). A harness that cannot fail would have passed the real run too.
3. **Findings that changed the plan before the platform run** (docs + local runs; the platform confirmed those that were tested, §13; the new platform-only findings are in item 1 and §13.6):
   - Browser **navigations** to `/auth/*` (the OIDC redirect back from Zitadel) are answered with `index.html` and never reach the
     Worker unless the path is in `assets.run_worker_first` — **[doc]** + **[measured-local]**. `/api` and `/api/*` are two different patterns.
   - The built-in SPA fallback is **one per Worker and always the root `/index.html`**, even under a prefix that has its own
     `index.html` — **[doc]** + **[measured-local]**. With `space` on its own hostname/Worker (ADR-0022) this problem disappears.
   - `/api/*` as an Access destination does **not** cover `/api` itself — **[doc]**. Both forms must be listed.
   - Worker-level Access (`worker` destination) answers WebSocket upgrades with **403** — **[doc]**. Path-scoped, hostname-based apps are the only usable shape.
   - A Worker is also reachable on `workers.dev`; hostname-based Access protects only the exact hostname — **[doc]**. `workers_dev=false` + `preview_urls=false` are mandatory.
   - Deleting a Custom Domain does **not** delete the edge certificate issued for it — **[doc]**. Teardown must remove it.
   - Bypass "does not enforce any Access security controls and requests are not logged" — **[doc]**. `/api` has no Access audit trail; ADR-0048 audit must come from the app.
   - In workerd with compat date >= 2026-03-17, **binary WebSocket frames arrive as `Blob`**, not `ArrayBuffer` — **[doc]** (`websocket_standard_binary_type`) and **[measured-local]**: my first echo silently sent the text `[object Blob]`. Yjs sync frames are binary: spikes 0001/0005 must set `ws.binaryType = "arraybuffer"` (DO hibernation handlers always receive `ArrayBuffer`).
   - Installed wrangler 4.123.0 supports compat dates only up to 2026-08-18; docs say "set to today". The spike pins `2026-08-18`. **[measured-local]**

Spike method step → state:

| Step | State |
|---|---|
| 1 hello Worker + path-scoped Access + WS echo | **done on the platform (run 2, §13)**: gated/bypass boundaries tested with must-fail controls, WS through bypass + gated + binding; one boundary leak found (case-insensitive bypass, BF3/T1) |
| 2 `.woven` via tunnel to Worker custom domain | **desk recipe only** (§7); owner decision needed |
| 3 verify from WARP client | not applicable until §7 is built |
| 4 topology comparison + service-binding latency | **measured on the platform (§13.5)**: hop +2.3-2.8 ms median, not zero; local numbers in §5 |
| 5 Q-0030.2 | desk answer (§4.1) |
| 6 public `space` host bypasses narrowly | **tested (SP1, SP2)**: the space host serves none of the product host's gated routes; SNI/Host mismatch is 403 |
| 7 static assets under prefixes (Q-0011.1) | answered by docs, local runs and the platform (S1-S5, §4.4, §13) |

## 1. What was built (`scripts/spike-0004/`)

```
dev.sh                  local-only wrangler dev (ROLE=edge|space); no credentials, isolated config dir
run_platform.sh         the real-platform runner: preflight deploy apply set-aud verify-present checks latency
                        teardown verify-absent scrub-state    ** nothing in it has been executed **
worker/edge/            JS Worker: static assets (/, /spaces/, /gated/), /api /auth echo, /live WS echo, /internal/* gated
                        routes with a secret marker, /api/lat/* latency probes, Access-JWT verifier (WebCrypto)
worker/b/               second Worker behind a service binding (fetch + RPC entrypoint; also a WS echo)
worker/space/           stand-in public `space` Worker: own hostname, no authenticated routes
worker/shared/logic.js  the compute() both sides run
tofu/                   throwaway OpenTofu root (provider cloudflare ~> 5, local backend, nothing else)
tests/run.py            CLI: checks | latency | selftest
tests/checks.py         the assertions (each positive paired with a must-fail control)
tests/rawhttp.py        raw HTTP/1.1 + RFC 6455 client (puts paths on the wire verbatim; stdlib only)
tests/mockgate.py       a MODEL of path-scoped Access with injectable defects (selftest only)
tests/latency.py        interleaved binding/rpc/in-process latency, p50/p95/p99, paired deltas with bootstrap CI
tests/jwt_verifier_test.py   local test of the Worker's Access-JWT verifier with throwaway RSA keys
tests/verify_teardown.py     GET-only inventory of spike-0004* leftovers, with a "verifier is blind" control
tests/delete_cert_packs.py   deletes the leftover edge certificate pack (dry-run by default)
```

Resources the OpenTofu root creates (all prefixed `spike-0004`): 2 Worker custom domains (`spike-0004.<zone>`, `spike-0004-space.<zone>`),
1 service token, 3 reusable policies (service-auth, UI allow, bypass), 4 Access applications (`ui` = whole host; `bypass` = `/api`,
`/api/*`, `/auth`, `/auth/*`, `/live`, `/live/*`; `space-bypass` = the space host; `overlap-gated` = `/api/gated`, `/api/gated/*`
nested under the bypass app). `tofu plan` must show exactly **10 to add**; `run_platform.sh apply` refuses to apply otherwise.

Resource names are Cloudflare provider v5.28.0 **[measured-local: `tofu init -backend=false` + `tofu validate` against the real provider schema]**:
`cloudflare_workers_custom_domain`, `cloudflare_zero_trust_access_application` (v5 uses `destinations = [{type = "public", uri = "host/path"}]`;
`domain`/`self_hosted_domains` are deprecated), `cloudflare_zero_trust_access_policy` (account-level reusable; `decision` = `allow|bypass|non_identity`
where `non_identity` is the UI's "Service Auth"), `cloudflare_zero_trust_access_service_token`. Valid `destinations.type` values include
`public`, `private`, `worker`, `preview_worker`, `all_workers`, `all_preview_workers` **[doc: github.com/cloudflare/terraform-provider-cloudflare docs/resources/zero_trust_access_application.md]**.
Other v5 Workers resources that exist: `cloudflare_workers_script` (accepts `assets.directory`, since provider 5.11.0 **[doc: developers.cloudflare.com/changelog/post/2025-10-09-assets-terraform/]**),
`cloudflare_worker` + `_version` + `cloudflare_workers_deployment`, `cloudflare_workers_route`, `cloudflare_workers_script_subdomain`.

## 2. What the local runs prove, and what they cannot

| Run | Result | Proves | Cannot prove |
|---|---|---|---|
| `tofu init -backend=false` + `validate` + `fmt -check` | clean | the root conforms to the real v5.28.0 schema (argument names, nesting, types) | enum values (`type = "public"`, `decision = "non_identity"` come from docs), anything the API rejects at apply |
| control for the above: add a bogus attribute | `validate` fails with "Unsupported argument" | a clean `validate` is not vacuous | — |
| `shellcheck` + `bash -n` on both shell scripts | clean | syntax and quoting | behaviour |
| `uv run tests/run.py selftest` (8 scenarios, below) | all 8 behaved as required | the harness reports PASS on a correct gate, FAIL on each injected defect, VOID when a must-fail control passes | anything about Cloudflare |
| `uv run tests/jwt_verifier_test.py` | valid token accepted; **10 controls rejected for the expected reason** | the Worker's RS256/kid/exp/iss/aud verifier works under workerd and rejects tampered payload, foreign aud, foreign iss, expired, `alg=none`, `alg=HS256`, unknown kid, wrong-key-same-kid, malformed, missing header (ADR-0050 Confirmation) | that real Access tokens verify (runbook G4); that the Rust Worker's verifier does (re-run the same vectors in spike 0003) |
| `verify_teardown.py --selftest`, `delete_cert_packs.py --selftest` | 5 + 1 cases right | the verifier tells present from absent from *blind*; cert deletion touches only all-spike packs | the real API response shape (written from docs) |
| raw client GETs to `developers.cloudflare.com` over TLS | 200, 210 KB chunked body, no hang; trailing-dot Host+SNI accepted by that edge | the TLS/chunked/SNI code path of the client | Access behaviour |

Selftest scenarios (target → required outcome), run against the real edge/B/space Workers under wrangler dev:

| Target | Required | Observed |
|---|---|---|
| ungated wrangler dev, `--expect-gate no` | functional PASS | PASS=21 INFO=1 SKIP=11 |
| same, but `--expect-gate yes` | gate controls **VOID** every dependent result | G0, B*, W1, T1, H1, M1 … VOID (17), nothing reported as PASS that depends on a gate |
| mock gate, correct | PASS | PASS=30; **T1: 105 probes, 0 leaks, 61 resolved to the gated target and were refused (non-vacuous), 44 inert** |
| mock `leaky` (matches raw path, forwards normalised) | T1 **FAIL** | FAIL, 30 probes leaked (`/api/../internal/worker-secret` …) |
| mock `dotleak` (trailing-dot Host matches no app) | H1 **FAIL** | FAIL (and SP2's dotted-Host variants) |
| mock `shallow` (`/x/*` covers one segment) | deep bypass paths **FAIL** | B3, B4, B7 + X1 FAIL (the latency probe path is deep) |
| mock `open` (gates nothing) | G0 **VOID**, nothing downstream trusted | VOID=29, only the non-gate X1 PASS |
| mock `spaceleak` (space host wired to the product Worker) | SP1 **FAIL** | FAIL, gated marker served via the space host |

Not validated negatively: SP2's SNI-vs-Host confusion (plain HTTP has no SNI, so the mock can only model Host-vs-Host); it shares its
leak detector with T1/H1, which are validated. Treat a first real SP2 PASS as the weaker of the two results.

### How a positive can be vacuous, and what the harness does about it
- **Gate-liveness control G0** (unauthenticated request to a gated path must be refused). If it passes, every bypass/traversal/WebSocket
  result is printed `*** CONTROL PASSED: RESULT VOID ***`, exit status 3.
- **Each gated positive** (service token) has two controls: no credentials refused, wrong secret refused.
- **Each bypass positive** has the string-prefix lookalike control (`/apix/ping`, `/authz`, `/liveness` must stay gated).
- **Each traversal probe is sent twice**, unauthenticated and with the token. "Refused unauthenticated" only counts as a pass if the
  token request shows the probe *does* resolve to the gated target; probes that never resolve are reported `inert`, and a run where
  none resolve is `WEAK`, not PASS.
- **Teardown verification** runs once expecting presence (must see what we created) and once expecting absence; an endpoint that
  errors (403 for a missing token permission) is `UNVERIFIABLE`, never "zero".

## 3. Facts from the documentation

### 3.1 Access path scoping and precedence  (source: https://developers.cloudflare.com/cloudflare-one/access-controls/policies/app-paths/ unless noted)

| Fact | Tag |
|---|---|
| An application is hostname + optional path. `example.com` (empty path) or `example.com/*` covers the apex and every path | [doc] |
| `example.com/alpha/*` covers `/alpha/one`, `/alpha/two` and **does not cover `/alpha`**, nor the apex | [doc] |
| A wildcard mid-path spans segments: `example.com/foo*/bar` covers `/foo/bar`, `/food/stuff/bar` | [doc] |
| At most one wildcard per slash level and per dot level | [doc] |
| Ports, query strings and `#` are **not supported** in application paths | [doc] |
| When rules overlap on a root path **the more specific rule wins and nothing is inherited** from the parent; with no specific rule the path inherits the parent's rules | [doc] |
| Overlapping *private* apps: most specific destination wins, creation order irrelevant | [doc: .../applications/non-http/self-hosted-private-app/] |
| Policy order inside an app: **Bypass and Service Auth first** (top to bottom), then Block/Allow in order; first Allow/Block match stops evaluation | [doc: .../access-controls/policies/] |
| Access is deny-by-default: a request matching no policy is blocked | [doc: reference-architecture/design-guides/designing-ztna-access-policies/] |
| **Not documented, now OBSERVED (run 2, §13)**: `host/api/*` **does cover deep paths** (`/api/a/b/c`, `/api/v1/work-items/123/comments?x=1`); `host/api` also matches `/api/` (trailing slash); the match is **case-INSENSITIVE** (`/Api/x` is a bypass; `/Api/gated/x` hit the nested gated app); the edge **normalises `..` and `%2e` dot-segments before Access matches** (so traversal out of a bypass prefix lands on the gated UI app), while `..;`, `%2f`, `%5c`, `%252e` stay under the prefix and are bypassed without reaching gated content; `%00` and `..` above root are `400`; trailing-dot / upper-case Host and `:443`/`:8443` stay gated; an SNI that differs from the Host header is `403` | [tested] -> B1-B12, BL1, T1, H1, SP2, BF3, O1 |

Consequence for the design: the UI app is the bare hostname; the bypass app lists both `host/api` **and** `host/api/*` (and the same for `/auth`, `/live`).
`host/api/*` is enough for deep API paths [tested], so the SPA's XHRs are not gated. The price is that the bypass is case-insensitive (BF3): see 13.4.

### 3.2 Bypass, service tokens, JWT  (sources: .../access-controls/policies/, .../service-credentials/service-tokens/, .../authorization-cookie/application-token/)

- Bypass: "does not enforce any Access security controls and requests are not logged"; it cannot use identity selectors; Bypass policies with device-posture rules **do not function when a Worker intercepts the request** — use Service Auth instead. **[doc]**
- Service tokens authenticate with `CF-Access-Client-Id` + `CF-Access-Client-Secret` and **need the Service Auth action**, otherwise Access prompts for an IdP login. **[doc]**
- **Strict service-token authentication** is on by default and not switchable for Zero Trust orgs created on/after 2026-10-05: failures are `401/403` instead of `302`; only Service Auth policies can authorise a request carrying service-token headers; Allow policies and any `CF_Authorization` cookie are ignored; no cookie is returned. **[doc]** The harness accepts both 302 and 401/403 as "refused". Whether service-token headers sent to a *Bypass* path change anything is finding BF2.
- Access adds a signed `Cf-Access-Jwt-Assertion`; validate signature, `aud` and `iss` against `https://<team>.cloudflareaccess.com/cdn-cgi/access/certs`; "validation of the header alone is not sufficient". **[doc]** **[tested, run 2]** a client-supplied `Cf-Access-Jwt-Assertion` is **not** stripped on a bypass path (it reaches the Worker), a client-supplied `Cf-Access-Authenticated-User-Email` **is** removed, and service-token headers sent to a bypass path reach the origin unstripped (BF1, BF2). On a GATED path the Worker received a genuine JWT that verified against the team JWKS (G4). ADR-0050's "the API Worker additionally verifies the Access JWT" therefore has to be **optional on `/api`, `/auth`, `/live`** (bypass paths carry no valid JWT) and **never trust `Cf-Access-*` headers there**.
- The edge Worker's `/internal/whoami` implements exactly that verifier (RS256 via WebCrypto, `kid` lookup, `exp`, `iss`, `aud`); set `TEAM_DOMAIN` and `POLICY_AUD` (the UI app's AUD tag, an output of the OpenTofu root) after `apply` (`run_platform.sh set-aud`).

### 3.3 WebSocket

- WebSockets are supported on all Cloudflare plans; only the initial upgrade is counted as a request; Cloudflare releases can terminate connections. **[doc: developers.cloudflare.com/network/websockets/]** A Worker WebSocket connection is billed as one request, messages are not requests. **[doc: .../workers/platform/pricing/]**
- **Worker-level Access policies do not support WebSocket: upgrade requests fail with 403**; protect WebSocket-using Workers with a hostname-based application. **[doc: .../workers/configuration/cloudflare-access/]** This is the only page I found that mentions Access + WebSocket (a search of the Cloudflare One docs index for "websocket" returned nothing); it does **not** state that hostname-based apps pass upgrades, it only recommends them. **[tested, run 2]** a hostname-based, path-scoped Access setup passes WebSocket upgrades: W1 on the BYPASS path `/live/echo` (101, text + binary + 64 KiB + 1 MiB echo, 50 sequential messages) and W2 on a GATED path `/internal/ws` with a service token (101, same echo; the Worker saw a valid Access JWT in the upgrade request, `jwt_present=True`); both with their must-fail controls (an upgrade on the gated path with no credentials and with a wrong secret is refused, never 101). W4, an upgrade forwarded through a service-binding hop to a second Worker, also works on the platform (256 KiB binary intact).
- Runtime updates give existing WebSocket connections up to 20 minutes to finish. **[doc: .../workers/platform/limits/]** A reconnect-on-close client is a requirement, not an option.
- `binaryType`: see §0. **[doc: .../workers/configuration/compatibility-flags/ "websocket_standard_binary_type", default as of 2026-03-17]**
- Whether an expiring Access session closes an established WebSocket is [unverified]; the bypass design (round 25) sidesteps it for `/live`.

### 3.4 Custom Domains, Routes, workers.dev  (sources: .../workers/configuration/routing/custom-domains/, .../routes/, .../cloudflare-access/)

- A Custom Domain makes the Worker **the origin** for the exact hostname: Cloudflare creates the DNS record and the certificate; **all paths** go to the Worker; no wildcard DNS; cannot be created on a hostname that already has a CNAME. **[doc]**
- Routes put a Worker *in front of* an existing origin: need a proxied DNS record, support path patterns (`*` only; no infix wildcards; path is case-sensitive; most specific wins), and **a route on the same hostname as a Custom Domain wins and can `fetch()` the Custom Domain Worker**. A route cannot be the target of a same-zone `fetch()`. **[doc]**
- Same zone: Worker -> Worker is only possible through a service binding, except that a Worker on a **Custom Domain** can be fetched directly. **[doc]**
- The Advanced Certificate generated for a Custom Domain needs no ACM subscription but is **not deleted** with the Custom Domain. **[doc]**
- Custom Domains need `Workers Routes Write` on each affected zone; they do not support per-Worker roles. **[doc: .../workers/authorization/workers/]**
- **Hostname-based Access protects only that exact URL; Worker-level Access protects every route, Custom Domain, `workers.dev` hostname and preview.** A Worker with `workers.dev` enabled is therefore reachable around every hostname app. **[doc]** -> `workers_dev:false`, `preview_urls:false` on all three Workers; check X2 requests the `workers.dev` URL and must not get the marker.
- Cost note: Worker -> Worker via a Custom Domain `fetch()` goes through the edge and is billed as a request; via a service binding it is not (§3.5).

### 3.5 Service bindings  (sources: .../runtime-apis/bindings/service-bindings/, .../workers/platform/pricing/, .../platform/limits/)

- "Zero overhead or added latency": both Workers run on the same thread of the same server by default. **[doc]** **[tested, run 2]** NOT reproduced: the paired median hop cost was +2.3 to +2.8 ms over the in-process path from this laptop, in two independent N=1000 passes, CIs excluding zero (§13.5). Local workerd showed +0.24 to +0.38 ms (§5).
- No extra request fees: one request is billed for the initial invocation, CPU time is the **sum across both Workers**. **[doc]**
- The six-simultaneous-connections limit is measured from the top-level request and **shared with Workers reached through bindings**. **[doc]**
- Local development with several Workers in one `wrangler dev` (`-c a -c b`) works but is documented as experimental. **[doc]** **[measured-local]** it worked on 4.123.0 for both `fetch` and RPC bindings.
- Workers VPC (beta) is the **opposite direction** (a Worker reaching a private network through a Tunnel). **[doc: developers.cloudflare.com/workers-vpc/]** It does not help a private name reach a Worker.

### 3.6 Static assets  (sources: .../workers/static-assets/*, .../wrangler/configuration/, .../platform/limits/)

- Default is asset-first: a matching file is served without invoking the Worker; static asset requests are free and unlimited. **[doc]**
- `assets.run_worker_first`: boolean or an array of glob patterns (`*`), `!` negative patterns win, order irrelevant, patterns must start with `/` or `!/`, **at most 100**, needs wrangler >= 4.20.0. `/api/*` does not match `/api`. **[doc]**
- `not_found_handling = "single-page-application"` serves **the root `/index.html`** for anything unmatched. **[doc]** **[measured-local]**: `/spaces/assets/nonexistent/deep/link`, `/spaces-not-the-prefix/deep` and `/nonexistent.js` all returned the root shell with 200, so a missing hashed asset is answered with HTML, not 404.
- With a Worker present, `not_found_handling` configured and compat date >= 2025-04-01, **navigation requests (`Sec-Fetch-Mode: navigate`) do not invoke the Worker**; the docs' own example is an OAuth callback. **[doc]** **[measured-local]** S5: with `run_worker_first` listing `/auth/*` the navigation reaches the Worker; `/authx/callback` (not listed) gets the SPA shell.
- One assets collection per Worker; assets served under a path prefix must be nested in a directory tree mirroring the path (`dist/blog/...` for `/blog/*`). **[doc: .../routing/advanced/serving-a-subdirectory/]**
- Limits: 20,000 files (Free) / 100,000 (Paid), 25 MiB per file; Worker size 64 MiB uncompressed, startup 1 s. **[doc: .../platform/limits/]**

### 3.7 Pricing facts used in §6.3  (source: https://developers.cloudflare.com/workers/platform/pricing/)

Workers Free: 100,000 requests/day, 10 ms CPU per invocation. Workers Paid: $5/month minimum for the account; 10 M requests/month included then $0.30/M; 30 M CPU-ms included then $0.02/M CPU-ms; no duration charge. **[doc]** Zero Trust Free plan "up to 50 users": **[unverified]** (the pricing page I could fetch did not state it; the owner confirms in the dashboard).

## 4. Desk answers

Convention: *Docs settle* = [doc]; *Measured* = [measured-local]; *Open* = needs the platform or Rust.

### 4.1 Q-0030.2 — do internal RPC routes and `/api/v1` share one Worker and one auth stack?

**Answer (desk): one `api` Worker, one auth stack, route groups inside one router.** The compat-plane routes, the core API and `/api/v1` are three
crates composed in one `thrust-worker` (ARCHITECTURE §2a); the permission seam sits in the use-case layer so every driving adapter is covered.
Per-group credential choice (session cookie vs `X-Api-Key`) is router middleware, not a Worker boundary.

- *Docs settle:* Access cannot distinguish the two surfaces and does not protect either: both sit under the bypass app's `/api/*` (round 25) and authenticate in-app, so sharing a path prefix and a Worker weakens nothing. Service bindings cost no latency and no request fee, so a split would not be *slower* — the argument for one Worker is deployables and the transitional `thrust-compat-plane` (deleted in Dioxus Phase 2), not performance. Bypass requests are not logged by Access, so the audit trail for both surfaces must come from the app (ADR-0048).
- *Open:* bundle size and cold-start of one Rust/Wasm Worker holding all three surfaces (64 MiB uncompressed, 1 s startup) — spike 0001. If the combined Worker breaches either limit, split `thrust-compat-plane` into its own Worker behind a service binding; the topology tests here carry over unchanged.
- Revisit if the owner wants third-party API traffic on its own hostname (`api.<zone>`): the Access bypass would become hostname-wide and the design changes (it also needs WAF/rate-limiting there, ADR-0033).

### 4.2 Q-0090 — one Worker with routes vs Worker-per-surface via service bindings

| | Option 1 single Worker | Option 2 role-split + bindings | Option 3 domain-split |
|---|---|---|---|
| Access path scoping | works | works | works. **Access keys on hostname + path, so it is independent of how many Workers sit behind it** [doc §3.1] |
| Hop cost | none | docs: zero overhead, no extra request fee, CPU summed [doc §3.5]; **platform: +2.3 ms (RPC) / +2.4-2.8 ms (fetch) median per hop, paired, N=1000 x2 passes, from a laptop** [tested, run 2, §13.5]; local workerd +0.24 / +0.38 ms [measured-local]. The documented request-fee claim is untested | same per hop, more hops |
| WebSocket across a hop | n/a | edge -> Worker B upgrade works, 256 KiB binary intact [measured-local W4]; the edge -> DO shape is untested | untested |
| Deploy/rollback | DO lifecycle coupled to every release (ADR text) | callee-first order; DO class isolated in `collab` | many skew combinations |
| Static assets | one collection, one SPA fallback | on the `edge` Worker | n/a |
| Public exposure | one `workers.dev` to disable | **every Worker must disable `workers_dev`/`preview_urls` individually** [doc §3.4]; Worker B has no public surface at all | N Workers to harden |

*Evidence added to ADR-0090:* the leading hypothesis (Option 2) is not contradicted by anything found; the hop is free per the docs, and Access scoping is
topology-independent. Confidence stays **medium** until the platform latency run and the Rust bundle sizes exist. One variation worth recording: a **route**
Worker (`host/api/*`) beside a Custom-Domain Worker on the same hostname takes precedence and can `fetch()` the Custom Domain [doc §3.4], which removes
the edge->api hop entirely at the cost of a second hostname-scoped deployable; with bindings being free, the simpler `edge` + bindings stays preferable.

### 4.3 Q-0090.2 — Custom Domains or Routes per host

**Answer (desk): Custom Domains for both product hostnames** (`<product>` and `space`): the Worker is the origin, DNS and the certificate are created
automatically, `cloudflare_workers_custom_domain` is a single resource, and nothing else owns the hostname. Use a Route only when a hostname must keep
another origin or a path must be carved out to a different Worker. Caveats from the docs: exact hostname only (no wildcard), no existing CNAME on the name,
`Workers Routes Write` per zone, and the certificate outlives the domain (§3.4). The spike's OpenTofu root already uses this shape.

### 4.4 Q-0011.1 — how are the apps built/prefixed under one Workers asset namespace, incl. SPA fallback?

**Answer (docs + local runs): there is no shared namespace to manage, because `admin` is retired (ADR-0021) and `space` has its own hostname and Worker (ADR-0022).**
Each Worker serves exactly one SPA at `/`, and plain `not_found_handling = "single-page-application"` is correct. Two rules are mandatory:

1. `run_worker_first` must list **both** forms of every dynamic path: `/api`, `/api/*`, `/auth`, `/auth/*`, `/live`, `/live/*` (and any other Worker-handled path).
   Otherwise a browser navigation to `/auth/callback` (the OIDC return from Zitadel) is answered with `index.html`. **[doc + measured-local]**
2. Content-hashed files should stay free static hits; a missing hashed asset is answered `200 text/html` by the SPA fallback (not 404), so deploy
   with version affinity and keep old hashes until clients age out (ADR-0090 already says so). **[measured-local]**

If the owner nevertheless wants both apps in one Worker under `/` and `/spaces/`: the built-in fallback is root-only, so a Worker handler is needed
(`worker/edge/src/index.js` `spaces()` maps extension-less misses to `/spaces/index.html`; `"!/spaces/assets/*"` keeps hashed files free), and the
`space` app must be **built with `/spaces/` as its base path**. Whether Plane's `space` app builds with a base path is [unverified]; ADR-0011 says it is built for its own hostname.

### 4.5 Access path-precedence rules, in one place

1. Destination forms: `host` = all paths; `host/p` = exactly `/p`; `host/p/*` = below `/p` but **not** `/p`. Mid-path `*` spans segments. No ports/query/`#`. **[doc]**
2. Overlap: the **more specific** application wins; **nothing is inherited** from the less specific one (so the bypass app does not inherit the UI app's policies, and a gated app nested under `/api/*` is not bypassed). **[doc]** Measured by O1 on the platform.
3. Inside the winning app: Bypass and Service Auth are evaluated before Allow/Block regardless of drag order; the first Allow/Block match decides. **[doc]**
4. Unmatched request: blocked. **[doc]**
5. Hostname-based apps protect exactly that hostname; `workers.dev` and previews need their own protection or must be disabled. **[doc]**
6. Everything in the "Not documented" row of §3.1 is open and has a check.

## 5. Latency harness, LOCAL numbers (workerd only; the platform numbers are in §13.5)

`uv run tests/run.py latency --base http://127.0.0.1:8787 -n 1000`, interleaved rotating order, one keep-alive connection, 100 warm-up, N=1000 per kind, wrangler dev on this Mac. **[measured-local]**

| kind | p50 ms | p95 ms | p99 ms |
|---|---|---|---|
| null (edge only) | 0.937 | 1.901 | 3.662 |
| in-process compute | 1.068 | 1.804 | 3.528 |
| service binding, fetch | 1.454 | 2.945 | 4.223 |
| service binding, RPC | 1.313 | 2.430 | 4.290 |

Paired difference per iteration (binding - in-process): median **+0.381 ms**, 95% bootstrap CI [+0.372, +0.389]; RPC - in-process **+0.240 ms** [+0.230, +0.250].
In-Worker loop (500 sequential calls, indicative): 0.464 ms/call fetch, 0.43 ms/call RPC.
Read this as: the harness resolves a sub-millisecond hop and works end to end. It is **not** evidence about production: workerd's multi-Worker dev mode is not the
same-thread production path, and the fetch path includes constructing a `Request` and parsing JSON that the in-process path skips. `Date.now()` in a Worker only advances
on I/O **[doc: .../workers/reference/security-model/]**, so a Worker-side stopwatch cannot time pure CPU and the client-side paired delta is the primary metric.

## 6. Runbook (written before the platform run; executed as run 2 in §13 with the deviations recorded there)

### 6.1 What the owner must supply

| # | Input | Notes |
|---|---|---|
| 1 | Go-ahead, a ~1 hour window, and the **sandbox account id** | env `SPIKE_ACCOUNT_ID` |
| 2 | **Zone id + zone name** of an active zone on that account | `SPIKE_ZONE_ID`, `SPIKE_ZONE_NAME`. No existing DNS record on `spike-0004.<zone>` or `spike-0004-space.<zone>` (a Custom Domain cannot be created over a CNAME [doc]). Free-plan zone is fine |
| 3 | **A 1Password item path holding an API token** | `SPIKE_OP_TOKEN_REF=op://<vault>/<item>/<field>`. Scope it to the sandbox account **and** the sandbox zone only, expiry <= 7 days. Account permissions: **Workers Scripts Edit, Access: Apps and Policies Edit, Access: Service Tokens Edit, Account Settings Read**. Zone permissions: **Workers Routes Edit, DNS Edit** (probably needed for the record a Custom Domain creates [unverified]), **SSL and Certificates Edit** (to delete the leftover edge certificate [doc]), **Zone Read**. Names from [doc: .../fundamentals/api/reference/permissions/] |
| 4 | **Zero Trust team name** (`<team>.cloudflareaccess.com`) | `SPIKE_TEAM_NAME`. The org must exist on the sandbox account |
| 5 | **A test identity** | `SPIKE_TEST_EMAIL`: an email that can complete the org's login (One-time PIN enabled, or an IdP) for the one manual browser step. The harness itself uses the service token, not this identity. Whether One-time PIN is enabled by default is [unverified] |
| 6 | The account's `workers.dev` subdomain (optional) | `SPIKE_WORKERS_DEV_HOST=spike-0004-edge.<subdomain>.workers.dev` enables check X2; without it X2 is SKIP and the `workers.dev` exposure goes unmeasured |
| 7 | Whether the org was created on/after 2026-10-05 | decides if strict service-token auth is forced on (affects 302 vs 401/403 only) |

Tools on the runner: `op`, `wrangler` (4.123.0 tested), `tofu`, `uv`.

### 6.2 Commands, in order, with expected outcomes

Run from `scripts/spike-0004/` after exporting the env above (none of it is a secret). The token is read by `op read` inside the script and never printed or put on argv.

| Step | Command | Expect |
|---|---|---|
| 0 | `./run_platform.sh preflight` | `preflight ok`; `tofu validate` success; verifier selftest ok |
| 1 | `./run_platform.sh deploy` | three `wrangler deploy`s (B, space, edge): "Uploaded", **no `workers.dev` URL** printed (disabled). Edge reports its 4 static assets. Deploy order matters: a binding target must exist first |
| 2 | `./run_platform.sh apply` | `Plan: 10 to add, 0 to change, 0 to destroy.` then `Apply complete! Resources: 10 added`. State is written to `~/.local/state/spike-0004/` (mode 700, **outside the worktree**). Allow a few minutes for the certificates to go active; early TLS errors are the cause, not a bug |
| 3 | `./run_platform.sh set-aud` | redeploys the edge with `POLICY_AUD` (UI app AUD tag) and `TEAM_DOMAIN`, enabling G4 |
| 4 | `./run_platform.sh verify-present` | CONTROL: `verifier sees every expected resource`. If it prints `VERIFIER BLIND`, fix token permissions **before** continuing, otherwise the final "zero" is meaningless |
| 5 | `./run_platform.sh checks` | writes `tests/out/checks.log` + JSON report; exit 0 only if no FAIL/VOID/WEAK. See §6.5 for how to read it |
| 6 | manual browser step | open `https://spike-0004.<zone>/` in a fresh profile, complete the Access login as the test identity, press **Run browser check**. Record `jwt_present` / `cf_authorization_cookie_present` reported for `/api/ping` (an ADR-0050 finding either way) and that the WebSocket opens and echoes |
| 7 | `./run_platform.sh latency` | `tests/out/latency.log`; p50/p95/p99 per kind and the paired binding-vs-in-process delta with CI; record in §5's table |
| 8 | `./run_platform.sh teardown` | `tofu destroy` (10 destroyed), `wrangler delete` x3, then the leftover edge certificate pack(s) deleted (only packs whose every host contains `spike-0004`) |
| 9 | `./run_platform.sh verify-absent` | `ZERO spike-0004 resources in every inventory (and every inventory answered)`; any `LEFTOVERS` or `VERIFIER BLIND / UNVERIFIABLE` line means teardown is **not** done |
| 10 | `./run_platform.sh scrub-state` | removes the local state (it holds the service-token secret) only after step 9 passed |

Teardown by hand if the script is unavailable: `tofu -chdir=tofu destroy`; `wrangler delete spike-0004-edge --force`, then `spike-0004-space`, then `spike-0004-b`;
dashboard **SSL/TLS > Edge Certificates**: delete the certificate packs whose hosts are `spike-0004*`; then run `verify_teardown.py --expect absent`.
Teardown covers: 3 Workers, 2 Custom Domains and their DNS records, 2 edge certificate packs, 4 Access applications, 3 policies, 1 service token.
Verification inventories exactly those seven resource types plus Workers routes (none expected).

### 6.3 Cost

- **The spike run itself: about $0 incremental.** Well under 1,000 requests for `checks` (about 250 probe requests plus the functional ones; WebSocket messages are not requests) and about 4,500 for `latency` (N=1000 x 4 kinds + warm-up) is far below Workers Free's 100,000/day; on a Workers Paid sandbox the 10 M included requests absorb it and the $5/month minimum is sunk. Service bindings add no request fee, static asset requests are free, custom-domain certificates need no ACM subscription. Resources exist for under a day. Zero Trust Free "up to 50 users" and its covering Access and service tokens: [unverified], the owner confirms in the dashboard. **[doc §3.7, §3.5, §3.4]**
- **Projected monthly cost of this topology at the sizing target (<=50 users, <=100k work items)** **[derived]**: assume 50 users x 20 days x 8 h x 720 API requests/h (one per 5 s: the P0 "refetch/poll" model) = **5.76 M requests/month**, 7 ms CPU average = 40.3 M CPU-ms. Requests: under the 10 M included -> $0. CPU: (40.3 - 30) M x $0.02/M = $0.21. Plus the $5 minimum -> **about $5.2/month**. A 3x chattier client (17.3 M requests, 121 M CPU-ms) gives $2.2 + $1.8 + $5 = **about $9/month**. Static assets, service-binding hops and the 3-4 Workers add $0. The Workers/edge part is nowhere near the < $100 envelope; the cost driver is the collab Durable Object duration (spike 0005), not this topology.
- **Effort:** run + read-out about half a day; ADR edits (0090/0091/0011/0050/0022, Q-0090.2, Q-0011.1, Q-0030.2) about half a day; re-run against the Rust Worker in wave 2 (the harness takes any `--base`).

### 6.4 Decision table: what each outcome means for the ADRs

| Check | If PASS | If FAIL / unexpected |
|---|---|---|
| B3/B4/B7 (deep paths under `/api/*`) | round-25 bypass design stands as written | the trailing wildcard is one level: list every depth, or move the API to its own hostname with a host-wide bypass (ADR-0090/0011/0050 amendment) |
| BL1 (lookalikes stay gated) | destinations do not over-match | a `/api*` style match exists: tighten destinations before anything ships |
| T1 / H1 / SP2 (traversal, Host/SNI) with non-vacuous resolution | bypass cannot be turned into gated access | **stop**: a gated route is reachable unauthenticated; do not ship path-scoped Access without app-level auth on every route |
| T1 `WEAK` | no probe form resolved to the target even with the token; assertion is weak | add probe forms the edge passes through before claiming safety |
| W1 / W2 / W4 | WebSocket survives bypass, gated and binding hops | W2 fail: keep `/live` on the bypass app (already the design); W1 fail: the collab design (ADR-0060) needs a different transport |
| O1 | specificity-wins and no-inheritance confirmed | nested gated apps cannot be used for sub-surfaces |
| S5/S2 | run_worker_first + prefix handling confirmed | adjust the Worker handler, not the topology |
| X2 | `workers.dev` serves nothing | `workers_dev=false` ineffective: use `cloudflare_workers_script_subdomain` (IaC) or Worker-level Access |
| BF1 | edge strips client-supplied `Cf-Access-*` on bypass paths | it does not: the API Worker must ignore the headers on `/api`,`/auth`,`/live` (ADR-0050 text) |
| SP1 | the space host serves nothing from the product Worker | the custom domain points at the wrong Worker: fix before ADR-0022 rolls out |
| G0 `VOID` | — | the gate is not enforcing: nothing else in the report can be trusted |

## 7. `.woven` half — **unverified, estate change needed** (desk recipe only; nothing was built or changed)

Question: how can a private `.woven` name reach a Worker through a tunnel origin pointing at the Worker's custom domain (owner direction, round 13; Q-0090.3)?

**What the estate rules fix [estate]:** a `.woven` name needs two legs (a Cloudflare hostname route and a CoreDNS rewrite). On woven-o11y EKS **both are created by annotating a K8s Service**
(`external-dns.alpha.kubernetes.io/hostname`); hand-writing either leg is a regression (409/1108 collisions), and P4 is reached so an OpenTofu-declared `.woven` host is not allowed. A Worker is **not** a Service,
so the annotation route has no object to hang off. The WARP path is **port-preserving L4**: cloudflared forwards to the backend on the port the client used, and a plain-HTTP backend on `:443` times out. The status prober is
a `non_identity` WARP session: Access-gating a probed `.woven` host breaks its probe permanently.

**Why "tunnel origin -> Worker custom domain" cannot be a bare pass-through [derived from §3.4 + estate rules]:** the client's TLS session for `x.woven` is carried end to end (L4) to whatever answers on the VIP. A Worker Custom Domain
answers only its exact public hostname and presents a Cloudflare-issued certificate for it; it cannot present a `x.woven` certificate, and it routes on SNI/Host. A connection that arrives with SNI/Host `x.woven` has nothing to match. So something
in the middle must **terminate TLS for `x.woven` and open a new TLS connection to the Worker's public hostname with that hostname as SNI and Host**.

**Recommended desk shape (needs an owner decision before anything is built):**

```
WARP client --(L4, :443, TLS: x.woven, org-CA leaf)--> tunnel --> K8s Service "worker-proxy" (annotated for external-dns => both legs created)
   nginx/envoy in woven-o11y: terminate :443 with a cert from ClusterIssuer agentic-org-ca (device-trusted),
   then proxy_pass https://<worker-custom-domain>:443  with  proxy_ssl_server_name on; proxy_ssl_name <host>; proxy_set_header Host <host>
   and inject Access Service-Auth headers (CF-Access-Client-Id/Secret from a K8s Secret)  --->  Worker custom domain (public hostname)
```

| Topic | Desk position |
|---|---|
| Hostname legs | none written by hand: annotate the proxy Service (existing pattern: `shared/kube/opencost/proxy.tf`, `products/cognee/kube/charts/cognee-proxy`). Create nothing in the shared tunnel or CoreDNS from this spike |
| Port | **:443**, TLS, read from the Service's `.spec.ports` when probing (a wrong-port probe is indistinguishable from an outage) |
| TLS chain | two legs: client <-> proxy with the org CA (verify with `/opt/homebrew/opt/openssl@3/bin/openssl s_client -CAfile <bundle> -verify_hostname x.woven`, **always paired with a must-fail control using an unrelated CA**; `curl --cacert` and `/usr/bin/openssl` pass against a wrong CA on this Mac); proxy <-> Worker with the public Cloudflare certificate (`proxy_ssl_verify on`) |
| Access interplay | the Worker's custom domain is **public**, so "private" is only true if that hostname is gated. Put an Access application with a **Service Auth** policy on the upstream public hostname and have the proxy present the service token (optionally also `Require` an IP range = cluster egress). The probed `.woven` name itself has **no** Access app, so `status.woven`'s non-identity WARP prober is unaffected |
| Dev-device identity | WARP enrolment is the access control for the `.woven` name; Access identity is not involved on the private leg |
| Probe design (when built) | assert **resolves + serves** from a WARP-enrolled client (`dig @127.0.2.2 x.woven +short` returns *some* address; never pin an IP or prefix), HTTP 200 from the Worker's JSON, **plus** controls: the public hostname without the token is refused, `x.woven` on the wrong port fails, and the Worker sees the cluster egress IP rather than the laptop's |
| Alternative B (no `.woven`) | a public hostname whose Access policy requires the `Warp` or `Gateway` selector (device connected to the Cloudflare One Client) **[doc: .../access-controls/policies/ selectors]**: private-by-policy without a tunnel. Different trust model from `.woven` (identity/posture, not network reachability); needs the owner to accept that `.woven` is not the mechanism |
| Alternative C | Workers VPC is Worker -> private network, the wrong direction **[doc]** |
| Owner decisions needed | (1) is a K8s proxy Service acceptable as the `.woven` identity of a Worker, or is Alternative B acceptable; (2) where the service-token Secret lives (ADR-0092); (3) which hostname is "internal" now that `apps/admin` is retired (ADR-0021) — Q-0090.1 says admin/internal hostnames are `.woven`, but the admin app no longer exists; (4) an infra-repo root for the proxy (not a lambda-admin k3s change) |

Unverified here: that cloudflared can reach a Cloudflare-proxied hostname from the cluster at all in this estate (egress path), that the proxy's re-origination sends cleanly through a Worker Custom Domain, and every probe above.

## 8. Risks: what I predicted would fail, and what actually did (runs 1-2)

The pre-run prediction list is kept so the outcome of each item is auditable.

| # | Predicted | Outcome |
|---|---|---|
| 1 | Trailing `/*` covers one level only; API gated | **did not happen** [tested]: `/api/a/b/c`, `/api/v1/work-items/123/comments?x=1`, `/auth/a/b/c` all reached the Worker (B3, B4, B7) |
| 2 | A traversal / encoding form reaches a gated route | **no traversal form leaked** [tested] (123 forms, 59 non-vacuous refusals). But a form I had not thought of did: **case variants of a bypass prefix** (`/Api/`, `/API/ping`) returned the gated root app shell unauthenticated (T1 FAIL on the extended set, BF3) |
| 3 | Edge propagation lag of minutes | **not observed**: Access enforced and both hosts served within the first second/minute (certificate issuance itself not measured, §13.7) |
| 4 | `workers_dev=false` ignored | **did not happen**: `workers.dev` and the version preview URL return 404 for all three Workers |
| 5 | Strict service-token mode changes 302 to 401/403 | not exercised: unauthenticated requests got `302` (UI app) or `403` (service-auth-only app); the service token worked everywhere it was allowed |
| 6 | Service-token `duration` string rejected | accepted (`168h`) |
| 7 | Access evaluates/strips service-token headers on a Bypass app | **it does not**: both headers reach the origin (BF2) |
| 8 | WebSocket through a gated app 403s or drops | worked (W2). The one drop seen was an 80 s idle-ish soak on an **ungated** bypass path in run 1 (W3); two 120 s soaks in run 2 were clean; cause unknown |
| 9 | `DNS Edit` needed for custom-domain creation | the custom domains were created with a token that has no DNS read; whether DNS write is held is unknown, but it was not a blocker |
| 10 | Rust Worker will differ (binaryType, 64 MiB/1 s) | not run (wave 2) |
| 11 | Local numbers are not production numbers | **confirmed**: local binding hop +0.24-0.38 ms vs platform +2.3-2.8 ms |

What actually failed or blocked, beyond that list:

1. **Run 1:** `POST /accounts/<id>/access/policies -> 403 auth.forbidden` (token lacked "Access: Apps and Policies Edit"); no Access assertion could run. Fixed by the owner between runs.
2. **Both runs:** `wrangler delete` with the API token fails (it needs KV read); the owner's OAuth login (with `CLOUDFLARE_ACCOUNT_ID` set to Woven) works. DNS-record and certificate-pack reads are 403 throughout.
3. **Run 2 first attempt:** `op read` failed twice with an authorization prompt/timeout (1Password locked); resolved by the owner unlocking.
4. **Documentation vs platform:** service-binding hop is not zero (+2.3-2.8 ms); the bypass match is case-insensitive.
5. **My own harness/script defects found while running** (all fixed in the scripts): the plan gate grepped coloured output; teardown stopped at the first failing delete under `set -e` (now continues and reports); the env file silently overrode an inline soak setting; the first 105 traversal forms had no case-variant bypass prefix (added after BF3); one `522` for a single probe in pass 3 was not reproduced or investigated.
6. **A genuine, reproducible security result:** T1 FAIL on `/Api/` and `/Api/internal/worker-secret` (200, root app shell, no credentials; no secret marker). See 13.3/13.4 and the recommendation there.

## 9. Not established (carried into the exit criteria) — updated after run 2

**Moved out of `[unverified]` by run 2 (observed, §13):** deep-path coverage of `/api/*`; `/api` vs `/api/`; path-case sensitivity (insensitive); dot-segment / percent-encoding handling; trailing-dot, upper-case Host and port variants; SNI-vs-Host mismatch; overlap precedence; client-supplied `Cf-Access-*` headers on a bypass path; service-token headers on a bypass path; WebSocket through a gated app, a bypass app and a service-binding hop; `workers_dev` / `preview_urls` off; `run_worker_first` navigation behaviour and prefix handling on the real platform; service-binding latency from outside (not zero); the 168 h service-token duration; the provider-v5 resource shapes.

**Still not established:**
(a) the *cause* of the one 80 s WebSocket drop (run 1, ungated) and of the unexplained gap between the in-Worker loop (3.8-5.0 ms per call) and the laptop-paired hop (2.3-2.8 ms); whether an Access session expiry closes an open gated WebSocket (the `/live` bypass design sidesteps it);
(b) whether the BF3 mitigation (reject / canonicalise non-canonical-case paths in the Worker before the static-asset fallback) works: **not deployed or tested**;
(c) certificate issuance time for a first-ever hostname (run 2 re-used what run 1 left), and whether the certificate packs are still in the account (unreadable);
(d) Access behaviour with an interactive IdP login (no browser login was performed; the harness used the service token only), strict service-token mode, Access audit logging on Bypass (docs say none), and a service-binding latency measured by something other than `Date.now()` inside a Worker;
(e) the `.woven` path (§7), Plane's `space` app building with a base path, Zero Trust seat pricing, the Rust Worker re-run (wave 2).

The spike's **exit criterion** ("a documented, reproducible recipe that serves a Worker on a `.woven` name and on a public name; topology chosen with measured overhead; Access/bypass boundaries tested with must-fail controls") is now **mostly met for the public half** — recipe, measured hop overhead and must-fail-controlled Access boundaries exist and ran on the platform, with one boundary (case-insensitive bypass match) found to leak the gated UI shell — and **not met for `.woven`**, which remains a desk recipe awaiting an owner decision.

## 10. As-run log (real platform, 2026-10-18) — PARTIAL, run halted

Tag `[tested]` = executed against the real Woven account in this session. Scratch paths below are under
`/private/tmp/claude-502/-Users-asmith-repos-woven-forks-plane/5dd6cc30-19bc-4362-9294-2aac5bb1243c/scratchpad/`
(`spike-0004.env` non-secret inputs, `spike-0004-baseline.json`, `spike-0004-state/` OpenTofu state **outside the plan package**, `spike-0004-out/` logs).
No token, service-token secret or Access JWT was printed, logged or written anywhere in the plan package.

### 10.1 Commands, in order (no secrets), wall clock UTC

| ~UTC | Step | Result |
|---|---|---|
| 07:02 | `run_platform.sh preflight` | ok (tofu validate, verifier and cert-delete selftests) |
| 07:03 | `token-verify` | `[tested]` `GET /accounts/<id>/tokens/verify` -> `success=true status=active expires_on=2026-11-18T04:59:59Z`; token length 53 |
| 07:03 | `snapshot baseline`, then `compare baseline` | `[tested]` baseline as announced, names exactly: Workers `art-filestore`, `workflows-starter-template`; custom domains `secure.gallery`, `upload.secure.gallery`; Access apps `App Launcher`, `Warp Login App`, `Proton SSH (infrastructure)`, `ds-cleaner SSH (infrastructure)`; service tokens `foo`, `status-page-prober`, `warp-fleet-lambda`; reusable Access policies 0; zone Workers routes 0. **`GET /zones/<zone>/dns_records` and `GET /zones/<zone>/ssl/certificate_packs` answer 403** (unverifiable). `compare` immediately before apply: identical |
| 07:04 | `deploy` | `[tested]` all three `wrangler deploy`s succeeded **with the API token** (Workers Scripts write works). Version ids: `spike-0004-b` 560b3b16-1edf-475b-bd81-94fdd038d693, `spike-0004-space` fb7260ec-6a14-4d00-a4b1-af33d517f7f7, `spike-0004-edge` c3ce482d-facb-4ee8-a58c-891d21d3ce6e. Wrangler printed "No targets deployed" for each (no `workers.dev`, no route yet) |
| 07:05 | `apply` (twice: the first two attempts failed in my own script, before any tofu apply: an ANSI-coloured plan defeated the `^Plan:` grep) | `[tested]` plan gate passed: **10 to add, 0 to change, 0 to destroy**, every resource `spike-0004*`, checked on the machine-readable plan. Apply then ran **partially** (below) |
| 07:06 | `wait-ready` (no-gate mode) | `[tested]` both hostnames answered over TLS at the **first** probe (`/api/ping` -> Worker JSON, space host -> space Worker JSON). The two custom-domain create calls had returned in 1-2 s and the first probe came about a minute later, so certificate/edge propagation was **under about two minutes**; finer resolution was not captured |
| 07:07 | `workers-subdomain` | `[tested]` account `workers.dev` subdomain is `woven-4d9` (so the edge would be `spike-0004-edge.woven-4d9.workers.dev`) |
| 07:08 | `checks` in ungated functional mode (`SPIKE_EXPECT_GATE=no`, with `--workers-dev-host`, `--space-host`, 120 s WebSocket soak) | executed; process exit status 1 (the one FAIL is W3). Log not read at the time (see 10.4); read afterwards with the coordinator's clearance, results in §10.5 (all from run 1; its resources are gone) |
| — | `latency`, `verify-present`, `teardown`, `verify-absent`, `scrub-state` | **not run** |

### 10.2 What was tested and what blocked

- `[tested]` Worker custom domains can be created with this token: `cloudflare_workers_custom_domain.edge` (id `22bf81ce2d9cb6e1ef6482466f96258e425cb7e6`) and `.space` (id `f92ace3f8f4458f06bda22f641dd368b8c099d05`) were created in 1 s and 2 s.
- `[tested]` An Access service token can be created: `cloudflare_zero_trust_access_service_token.test` (id `3e69a148-9be4-4279-85e2-fffd5be5a23c`, duration `168h` accepted). So **Access: Service Tokens Edit** is held.
- `[tested]` **BLOCKED:** creating the three reusable Access policies failed identically:
  `POST https://api.cloudflare.com/client/v4/accounts/<cloudflare-account-id>/access/policies -> 403 Forbidden {"success":false,"errors":[{"code":1010,"error":"auth.forbidden"}]}`
  The four Access applications depend on those policies and were never attempted. **Missing permission: account-level "Access: Apps and Policies Edit"** (the token reads Access apps, per the pre-flight, but cannot write policies; app creation needs the same permission). No other credential was sought, as instructed.
- Consequence: **no Access-dependent assertion ran**: gated vs ungated, bypass vs gated precedence, deep-path coverage of `/api/*`, traversal/encoding/trailing-dot/case/SNI-vs-Host probes, WebSocket through a gated app, the overlap experiment, and the Access JWT/header findings are all still `[unverified]`. Nothing in §0-§9 moves out of `[unverified]` on the strength of this run.
- Items that need no Access (WebSocket through the bypass path and a service binding, `run_worker_first` navigation behaviour, SPA/prefix behaviour, `workers.dev` off, binding latency) were queued in the ungated functional run and in `latency`; the functional results are in §10.5, the latency run never started.

### 10.3 Resources the first run left in the Woven account (HISTORICAL: torn down afterwards, see §11; all `spike-0004*`, nothing else was touched)

| Resource | Name / id | Created by |
|---|---|---|
| Worker | `spike-0004-b`, `spike-0004-space`, `spike-0004-edge` | `wrangler deploy` |
| Worker custom domain | `spike-0004.sow.foo` (id 22bf81ce2d9cb6e1ef6482466f96258e425cb7e6) -> `spike-0004-edge`; `spike-0004-space.sow.foo` (id f92ace3f8f4458f06bda22f641dd368b8c099d05) -> `spike-0004-space` | tofu |
| DNS records | the two records Cloudflare creates for those custom domains (unverifiable: DNS read is 403) | the custom domains |
| Edge certificate packs | one per custom domain (docs: **not deleted** with the domain); unverifiable: certificate-pack read is 403 | the custom domains |
| Access service token | `spike-0004-test-token` (id 3e69a148-9be4-4279-85e2-fffd5be5a23c) | tofu |
| Access policies / applications | **none created** | — |

Exposure while it stays up: `https://spike-0004.sow.foo/` is **publicly reachable with no Access in front of it** (no Access app exists); it serves the test Worker, whose "gated" routes return fixed marker strings only (no real secret exists anywhere in the spike). Baseline resources are unchanged as of the last `compare` (pre-apply); a post-apply `compare` was not run.

### 10.4 Why the run stopped, and how to finish it

While waiting for the ungated checks to finish the harness's wait command was denied by the permission system ("Protected-Scope IaC Apply"). I treated that as a stop condition, did **not** read the checks log by another route, and made no further calls against the Woven account. The coordinator's message authorising the run is not itself consent; that decision belongs to the owner.

To finish (owner decision): either re-run after the token gains **Access: Apps and Policies Edit** (the apply is idempotent: `./run_platform.sh apply` will plan only the 7 remaining adds, which the script's 10-add gate will refuse, so change the gate or destroy first), or tear down now:

1. `SPIKE_ENV_FILE=<scratch>/spike-0004.env ./run_platform.sh teardown` — `tofu destroy` (3 resources in state: 2 custom domains, 1 service token), `wrangler delete` x3 (token auth; if the token lacks delete, `SPIKE_WRANGLER_AUTH=oauth` with `CLOUDFLARE_ACCOUNT_ID=<cloudflare-account-id>`). The last sub-step, `delete_cert_packs.py`, will report it cannot list certificate packs (403): **delete the packs whose hosts are `spike-0004.sow.foo` and `spike-0004-space.sow.foo` by hand in the dashboard (SSL/TLS > Edge Certificates)**, or give the token "SSL and Certificates Edit".
2. `SPIKE_ENV_FILE=<scratch>/spike-0004.env ./run_platform.sh compare baseline` — must report identical to the pre-apply snapshot and no `spike-0004` names (DNS and certificate packs stay unverifiable with this token; the two Workers/custom-domain lists are the evidence).
3. `./run_platform.sh verify-absent` then `scrub-state` (the state under `spike-0004-state/` holds the service-token secret).
4. The ungated functional log is at `<scratch>/spike-0004-out/checks-gate-no.log` (JSON report beside it) for whoever is cleared to read it.

### 10.5 What the first run's ungated functional log shows (run 1, real edge, NO Access in front; resources since destroyed)

Source: `<scratch>/spike-0004-out-run1/checks-gate-no.log` (JSON beside it). `[tested-run1]` = executed against the real Cloudflare edge in run 1.
Because no Access app existed, **gate controls were not applicable** and every result below says something about Workers / the edge / my Worker code only, never about Access.
Summary line of that run: `FAIL=1 PASS=26 INFO=1 SKIP=8`.

| Check | Result | What it establishes (and does not) |
|---|---|---|
| B1-B12 (`/api`, `/api/`, `/api/ping`, `/api/a/b/c`, `/api/v1/.../comments?x=1`, same for `/auth*` and `/live*`) | PASS | `[tested-run1]` the Worker answers every one of those paths, incl. the exact and trailing-slash forms and deep paths: my `run_worker_first` pattern list is right on the real platform. Says nothing about Access matching those paths |
| W1 WebSocket on `/live/echo` | PASS | `[tested-run1]` upgrade + text, binary, 64 KiB and 1 MiB echo + 50 sequential messages through the real edge |
| W2 WebSocket on `/internal/ws` | PASS (ungated) | `[tested-run1]` same, on the "gated" route **with no gate**: does not show WebSocket through an Access-gated app |
| W4 WebSocket through a service binding (edge -> Worker B) | PASS | `[tested-run1]` 101 via the binding, Worker B answered, 256 KiB binary intact: the edge -> binding -> second-Worker WebSocket shape works on the platform |
| **W3 soak: message every 20 s for 120 s on `/live/echo`** | **FAIL** | `[tested-run1]` **the connection closed before 80 s ("dropped after 80s: WSError: connection closed") although a message was echoed every 20 s.** Ungated, plain Worker (no Durable Object), so not Access. Cause **[unverified]** (edge idle policy, runtime limit and a harness artefact are all possible; the harness had echoed at 20, 40, 60 s). Retest with a faster ping interval and a hibernating Durable Object before relying on a long-lived socket; a reconnect-on-close client is required either way (docs already say runtime updates can end connections) |
| S1-S5 (root shell, deep link, `/spaces/` prefix + deep link, hashed asset, control path outside the prefix, navigation to `/auth/callback`) | PASS | `[tested-run1]` on the real platform: SPA fallback serves the ROOT shell, the Worker handler gives `/spaces/*` its own shell, hashed assets are served as assets, and a **navigation** to `/auth/callback`, `/auth`, `/api` reaches the Worker because of `run_worker_first`. The S4/S5 controls held (outside-prefix path gets the root shell; unlisted navigation path gets the SPA shell). This moves §3.6/§4.4 from "doc + measured-local" to tested |
| X1 binding fetch / binding RPC / in-process give the same hash | PASS | `[tested-run1]` same logic in a second Worker over a service binding on the platform |
| X2 `https://spike-0004-edge.woven-4d9.workers.dev/internal/worker-secret` | PASS | `[tested-run1]` `404 not-found`: with `workers_dev: false` the edge Worker is not served on `workers.dev` (wrangler also printed "No targets deployed") |
| SP1 public space host | PASS (ungated) | `[tested-run1]` the space hostname answered with the space Worker, and `/internal/worker-secret`, `/gated/asset-secret.txt`, `/internal/ws` on it were all 404 with no product marker: the separate Worker really has no gated routes. Says nothing about Access narrowness |
| BF1 forged `Cf-Access-Jwt-Assertion` + forged `Cf-Access-Authenticated-User-Email` on `/api/ping` | INFO | `[tested-run1]` **the Worker saw the forged `Cf-Access-Jwt-Assertion` (`jwt_present=True`) but NOT the forged `Cf-Access-Authenticated-User-Email` (`email_header_present=False`).** So on the real edge, with no Access app, a client-supplied JWT header reaches the Worker while the email header is removed; locally (workerd) both reached it. To be re-measured with Access ON before it is used in an ADR (Access may behave differently on a bypass path) |
| G1-G3 | PASS (ungated) | `[tested-run1]` markers served by the dynamic route, the static asset and the root shell; with no gate there is nothing to refuse |
| SKIP | BL1, T1, H1, M1, SP2, G0, G4, O1 | need the gate; **still `[unverified]`** |

Not yet measured anywhere on the platform: latency (run 1 never reached it).

## 11. Teardown of run 1 (run by the coordinator after the owner chose "Tear down now", 2026-10-18; run 2's teardown proof is §14)

- `[tested]` `run_platform.sh teardown`: `tofu destroy` removed the 3 tofu resources (2 custom domains, the service token). The first `wrangler delete`
  with the API token failed with an authentication error on a KV-namespace list (**the token can deploy Workers but cannot delete them**: it lacks KV read,
  which `wrangler delete` needs). The script's `set -e` also stopped before the cert-pack step.
- `[tested]` Remaining Workers were deleted with the owner's wrangler OAuth login (`CLOUDFLARE_ACCOUNT_ID` set to Woven, `CLOUDFLARE_API_TOKEN` unset):
  `spike-0004-edge` was already absent when the OAuth retry reached it (deleted by the first attempt before it failed), then `spike-0004-space` and `spike-0004-b` deleted.
- `[tested]` `verify-absent`: zero `spike-0004` in Workers scripts, custom domains, Workers routes, Access apps, Access policies and Access service tokens.
  `compare baseline`: every readable inventory is identical to the pre-apply baseline (2 Workers, 2 custom domains, 4 Access apps, 3 service tokens, 0 policies, 0 routes).
- `[tested]` Neither `spike-0004.sow.foo` nor `spike-0004-space.sow.foo` resolves any more (`dig +short` empty; `curl` could not resolve the host), so Cloudflare removed the
  DNS records with the custom domains. The DNS-record inventory itself stays unverifiable (403).
- **Left behind, unverified:** the edge certificate packs for the two hostnames (the platform keeps them after a custom-domain delete; the token cannot list or delete them, 403).
  The cert-delete step was never reached. **Owner action:** delete the two packs in the dashboard (SSL/TLS > Edge Certificates), or give the token SSL and Certificates Edit and I re-run `delete_cert_packs.py`.
- `[tested]` `scrub-state`: local OpenTofu state, plan and data dir removed (they held the service-token secret). The ungated functional log was **not read**.
- **Token gaps found by this run:** missing Access: Apps and Policies Edit (blocks the whole Access half), SSL and Certificates read/Edit, DNS read, KV read (blocks `wrangler delete` with the token).
  Held: Workers Scripts write, Custom Domains write, Access Service Tokens write, Account/Zone read.

## 12. Run 2, first attempt (after the owner added "Access: Apps and Policies Edit"): NOT STARTED — blocked before any remote call (superseded by §13)

- 2026-10-18 ~08:43-08:45 UTC. Local preparation done: `run_platform.sh teardown` hardened (every sub-step runs even if an earlier one fails or its target is already gone; the three `wrangler delete`s are a fixed loop over `spike-0004-edge`, `spike-0004-space`, `spike-0004-b` and use `SPIKE_WRANGLER_AUTH=oauth` when set; the exit status reports if any sub-step failed); run 1's logs archived to `<scratch>/spike-0004-out-run1/`; local OpenTofu state confirmed absent.
- `run_platform.sh token-verify` is the first step and reads the token with `op read 'op://<vault>/<item>/token'`. It failed **twice**: `error initializing client: authorization prompt dismissed, please try again`, then (plain retry) `error initializing client: authorization timeout`. By the estate's 1Password rule that means the desktop app is locked or the approval prompt went unseen; it is not a Cloudflare 403 and not a harness denial. **No Cloudflare call was made in run 2** (no snapshot, no deploy, no apply). No other credential was tried.
- To proceed: the owner unlocks 1Password and approves the `op read` prompt, then run 2 starts from the top (`snapshot`/`compare` baseline, `deploy`, `apply` with the 10-add gate, `wait-ready`, `set-aud`, `verify-present`, `checks` with `SPIKE_EXPECT_GATE=yes`, `latency`, `teardown` with `SPIKE_WRANGLER_AUTH=oauth`, `verify-absent`, `compare baseline`, `scrub-state`). Nothing in §0-§9 has moved out of `[unverified]` beyond what §10.5 records.
- **Still standing in the account** (per §11, unverified because the token cannot read or delete them): the two edge certificate packs for `spike-0004.sow.foo` and `spike-0004-space.sow.foo`.

## 13. Run 2 (real platform, 2026-10-18, all with the gate ON): as-run log and results

Tags: `[tested]` = executed against the real Woven account in this run; `[tested-run1]` as in §10.5. Scratch dir `<scratch>` = `/private/tmp/claude-502/-Users-asmith-repos-woven-forks-plane/5dd6cc30-19bc-4362-9294-2aac5bb1243c/scratchpad/`
(`spike-0004.env` non-secret inputs; `spike-0004-baseline.json`; `spike-0004-state/` OpenTofu state, outside the plan package; `spike-0004-out/` logs and JSON reports, `*.pass1/2.*` are kept copies).
No token, service-token secret or Access JWT was printed, logged or written into the plan package.

### 13.1 Commands, in order (UTC from `date -u`; no secrets)

| ~UTC | Step | Result |
|---|---|---|
| 13:40 | `token-verify` | `[tested]` account-owned token active, length 53, expires 2026-11-18 (the 1Password prompt that timed out in §12 was approved this time) |
| 13:40 | `snapshot run2-pre`, `compare run2-pre` | `[tested]` Workers `art-filestore`, `workflows-starter-template`; custom domains `secure.gallery`, `upload.secure.gallery`; 4 Access apps; 3 service tokens; 0 Workers routes. **Access policies: 7 pre-existing reusable policies are now visible** (`CF Admins`, `Lambda fleet service token (WARP enrollment)`, `ML Team`, `Read Only Admins`, `Status-page prober service token (WARP enrollment)`, `Woven Emails`, `Woven Onboarding`) where run 1 listed 0: the policy list was empty to the token before it held Access read for policies, so the earlier "0 policies" baseline was an artefact of permissions, not of the account. None were created by this spike and none were touched. DNS-record and certificate-pack reads still 403 (unverifiable). Compare against snapshot and baseline file: identical |
| 13:41 | `deploy` | `[tested]` three `wrangler deploy`s with the API token. Versions: `spike-0004-b` d1c00725-8dfb-4b8a-901f-faf53755d7f9, `spike-0004-space` 47aba861-78d2-455b-99d3-b5f635752c91, `spike-0004-edge` 93f372f6-e40d-4402-b445-b889289e8d9e |
| 13:41 | `apply` | `[tested]` plan gate on the machine-readable plan: **10 to add, 0 change, 0 destroy, every resource carries `spike-0004`**; then `Apply complete! Resources: 10 added`, every create returned in under 1 s. **The Access policy POST that returned 403 in run 1 now succeeds** (the token's "Access: Apps and Policies Edit" works) |
| 13:41 | `wait-ready` | `[tested]` gate ON: at t+0 s the bypass path answered Worker JSON, the gated path answered **302 to Access**, the space host answered. Ready after the 2-consecutive-ok rule (6 s). **Certificate and Access propagation: none observable** (see 13.7: the certificate packs from run 1 were still in place, so this is NOT a fresh-certificate measurement; the Access apps took effect inside the same second as their creation) |
| 13:41 | `set-aud` | `[tested]` edge Worker redeployed with `TEAM_DOMAIN`=`https://woven-solutions.cloudflareaccess.com` and `POLICY_AUD` = the UI app's AUD tag (both shown as `(hidden)` by wrangler); version 98127868-1891-49b4-8a38-6dc5278a36ff |
| 13:42 | `verify-present` (control) | `[tested]` the inventory tool saw 3 Workers, 2 custom domains, 4 Access apps, 3 policies, 1 service token named `spike-0004*`; DNS and certificate packs unverifiable (403, allowed list). Control held: the later "zero" is meaningful for the six readable inventories |
| 13:42-13:45 | `checks` pass 1 (gate ON, 120 s WebSocket soak, `--workers-dev-host`, `--space-host`, `--overlap`) | `[tested]` **PASS=35, INFO=2, FAIL/VOID/WEAK = 0**; harness exit 0 |
| 13:46-13:48 | `latency` pass 1 (N=1000 per kind, 100 warm-up, from this laptop) | `[tested]` §13.5 |
| 13:48-13:51 | `checks` pass 2 (after adding BF3 and the BF2 secret-header detail) | `[tested]` PASS=35, INFO=3, no FAIL/VOID/WEAK: **every pass-1 result reproduced**; W3 (the soak) passed again |
| 13:52-13:55 | `checks` pass 3 (after adding 18 case-variant-prefix + traversal forms to T1: 123 probe forms instead of 105) | `[tested]` **PASS=34, INFO=3, FAIL=1: T1 FAIL** (2 probes leaked the root app shell via case-variant bypass prefixes, §13.3/13.4); all other assertions reproduced a third time |
| 13:56-13:58 | `latency` pass 2 | `[tested]` §13.5; reproduces pass 1 |
| 13:59 | preview/`workers.dev` probes | `[tested]` `https://98127868-spike-0004-edge.woven-4d9.workers.dev/internal/worker-secret`, `https://spike-0004-edge.woven-4d9.workers.dev/internal/worker-secret`, `https://spike-0004-b.woven-4d9.workers.dev/`, `https://spike-0004-space.woven-4d9.workers.dev/` all -> `404 text/plain`: no Worker served on `workers.dev` or on the version-prefixed preview URL (`preview_urls: false`, `workers_dev: false`) |
| 13:59 | `teardown` with `SPIKE_WRANGLER_AUTH=oauth` (hardened script) | `[tested]` see §14 |
| 14:00-14:01 | `verify-absent`, `compare run2-pre` (= baseline), authoritative-DNS checks, `scrub-state` | `[tested]` see §14 |

Run 2 wall clock: 13:40:08 -> 14:01:07 UTC, about 21 minutes including three checks passes and two latency passes.

### 13.2 Resources created in run 2 (non-secret ids, so teardown is exact)

| Resource | Name | Id |
|---|---|---|
| Worker custom domain | `spike-0004.sow.foo` -> `spike-0004-edge` | 22bf81ce2d9cb6e1ef6482466f96258e425cb7e6 (identical to run 1: the id is derived from the hostname, not random) |
| Worker custom domain | `spike-0004-space.sow.foo` -> `spike-0004-space` | f92ace3f8f4458f06bda22f641dd368b8c099d05 (same as run 1) |
| Access service token | `spike-0004-test-token` | 0d1df20a-1854-40a3-9b7b-6595002e9388 (client id `1350ef6f02b0ed8b033c253de668329b.access`; the secret stays in the state file only) |
| Access policy | `spike-0004-service-auth-test-token` (decision `non_identity`) | f48cc8ec-be52-4228-927b-4610fe928a07 |
| Access policy | `spike-0004-ui-allow-test-identity` (allow, `<operator-email>` only) | 4c0adafc-6f5a-457e-a967-132761b8820e |
| Access policy | `spike-0004-bypass-everyone` (bypass) | 9359901f-ddf8-4551-a583-c31514d93562 |
| Access app | `spike-0004-ui` (whole host; policies: service-auth then allow) | 5ad15f4f-8b19-48dd-8e18-a433f33f26f8 |
| Access app | `spike-0004-bypass` (`/api`, `/api/*`, `/auth`, `/auth/*`, `/live`, `/live/*`) | b2de8358-79b2-4fa6-8175-36dfc8e4d3d4 |
| Access app | `spike-0004-space-bypass` (whole `spike-0004-space.sow.foo`) | fb489595-ac89-4a0f-a0f2-00d5dcb78368 |
| Access app | `spike-0004-overlap-gated-under-api` (`/api/gated`, `/api/gated/*`; service-auth only) | 42fddfab-cce6-4e62-b3ce-855108b7e0eb |
| Workers | `spike-0004-b`, `spike-0004-space`, `spike-0004-edge` | versions in 13.1 |
| DNS records, edge certificate packs | created by Cloudflare for the two custom domains | not listable with this token (403) |

The `ui` app and the `bypass` app have different AUD tags (apply outputs); the edge Worker verifies `aud` against the UI app's tag (check G4).

### 13.3 Per-assertion results (real platform, gate ON); "control" = the paired request that MUST be refused or must fail

No assertion was VOID (no must-fail control passed in any of the three passes) and none was WEAK. **One assertion FAILED: T1, in pass 3 only** (the extended probe set). Detail lines are from `<scratch>/spike-0004-out/checks-gate-yes.pass1.log`, `.pass2.log` and `.pass3.log`; pass 3 (123 T1 forms, otherwise the same checks) reproduced every other result, so each PASS below was observed three times.

| ID | Claim | Result | Must-fail control(s) and what they did |
|---|---|---|---|
| G0 | an unauthenticated request to a gated path is refused | PASS | is itself the gate-liveness control: dynamic route and static asset both `302` to Access |
| G1 / G2 / G3 | gated dynamic route / gated **static asset (Worker not invoked)** / root shell accept the service token | PASS | no credentials -> refused; wrong secret -> refused (both held) |
| G4 | a gated path hands the Worker a valid Access JWT | PASS | no credentials on `/internal/whoami` refused. The Worker verified the JWT with the **real team JWKS**: `{valid: true, reason: ok, type: app}` (RS256, `kid`, `exp`, `iss`, `aud`) |
| B1-B12 | `/api/ping`, `/api`, `/api/`, `/api/a/b/c`, `/api/v1/work-items/123/comments?x=1`, `/auth/callback`, `/auth`, `/auth/`, `/auth/a/b/c`, `/live`, `/live/`, `/live/echo` reach the Worker with **no credentials and no Access JWT** | PASS x12 | G0 held (gate live); the string-prefix lookalike control held (`/apix/ping`, `/authz` and `/liveness` stay gated) |
| BL1 | bypass destinations do not over-match by prefix (`/apix`, `/api-v2`, `/authz`, `/authx/y`, `/liveness`, `/apiary/x`) | PASS | G0 held; all six were `302` to Access |
| T1 | traversal / encoding from a bypass prefix toward gated targets never returns gated content unauthenticated | **PASS on the 105 forms of passes 1-2; FAIL on the extended 123-form set (pass 3): 2 leaks** | G0 held in all three passes; every probe was also sent **with** the token to prove it was not inert (pass 1/2: 57 of 105 resolved to the gated target and were refused unauthenticated, 48 inert). **The 2 leaks (pass 3) are `/Api/` and `/Api/internal/worker-secret`: 200 with no credentials, body = the 1,712-byte root app shell (`ROOT-SHELL` marker only; neither secret marker appears).** Cause = BF3 (case-insensitive bypass match + case-sensitive origin + SPA fallback). The traversal forms themselves leaked nothing; the 18 new case-prefix forms are tabulated in 13.4. The first 105 forms contained no case variant of a bypass prefix, which is why passes 1-2 did not see it |
| H1 | trailing-dot Host, upper-case Host, `:443`, `:8443`, Host+SNI trailing dot, upper-case SNI do not bypass the gate | PASS | G0 held; 6 of 6 resolved to the gated target with the token and were `302` unauthenticated (non-vacuous) |
| M1 | OPTIONS / HEAD / POST / PUT / DELETE to a gated path are refused | PASS | G0 held. OPTIONS -> `403`, the others `302` |
| W1 | WebSocket upgrade, text/binary/64 KiB/1 MiB echo, 50 sequential messages on the BYPASS path `/live/echo`, no credentials | PASS | G0 held; an upgrade on the gated `/internal/ws` with no credentials was refused (not 101) |
| W2 | the same through a GATED path with the service token | PASS | no credentials -> upgrade refused; wrong secret -> refused. The Worker saw `jwt_present=True` on the upgraded request |
| W4 | upgrade through a service-binding hop (edge -> Worker B), B answers, 256 KiB binary intact | PASS | G0 held; the edge's own echo is distinguishable from B's (`echo:` vs `b-echo:`) |
| W3 | WebSocket on the bypass path survives a 120 s soak (a message every 20 s) | PASS in pass 1 and pass 2 (6 messages each) | G0 held. **Run 1 (ungated) failed the same test at 80 s**; two later passes of 120 s did not. Not reproducible, cause unknown (`SPIKE_WS_SOAK` stayed 120 because the env file overrides inline values) |
| S1-S4 | root shell + deep link; `/spaces/`, `/spaces` and a deep link serve the **spaces** shell; hashed asset served as an asset; a path outside the prefix gets the root shell | PASS | unauthenticated requests refused (302) in every case; S4 is the distinguishability control for S2 |
| S5 | a browser **navigation** to `/auth/callback`, `/auth`, `/api` reaches the Worker (`run_worker_first`), not `index.html` | PASS | G0 held; the control (navigation to a path **not** in `run_worker_first`, `/authx/callback`) did receive the SPA shell |
| X1 | binding fetch, binding RPC and in-process return the identical result from a second Worker | PASS | different input gives a different hash (the equality can fail) |
| X2 | `https://spike-0004-edge.woven-4d9.workers.dev/internal/worker-secret` does not serve the gated route | PASS | `404` there while the custom domain served the marker with the token (the target exists); G0 held |
| O1 | overlap: a **gated** app at `/api/gated`, `/api/gated/*` nested under the **bypass** app's `/api/*` wins, nothing inherited | PASS | G0 held; sibling `/api/ping` was still bypassed. Unauthenticated `/api/gated`, `/api/gated/x`, `/api/gated/x/y/z` -> `403` (not `302`: that app has only a Service Auth policy, so there is no login to offer); with the token -> `200` |
| SP1 | the public `space` host answers with no credentials and serves none of the product host's gated content | PASS | G0 held; `/internal/worker-secret`, `/gated/asset-secret.txt`, `/internal/ws` on the space host were `404` with no product marker |
| SP2 | SNI/Host confusion between the two hostnames does not expose gated content | PASS (weaker than T1/H1) | 12 probes, 0 leaks, but only 3 resolved to the gated target; the 9 genuinely mismatched ones (SNI one host, Host the other, either direction) were all `403` |
| BF1 (finding) | client-supplied `Cf-Access-Jwt-Assertion` and `Cf-Access-Authenticated-User-Email` sent to a bypass path | INFO | **the forged JWT header reaches the Worker (`jwt_present=True`); the forged email header does not** (`email_header_present=False`). Same as run 1 (no Access then), so Access did not change it |
| BF2 (finding) | service-token headers sent to a bypass path | INFO | `Cf-Access-Client-Id` **and** `Cf-Access-Client-Secret` both reach the origin; no JWT is added. Bypass does not consume or strip credentials: a client that attaches its service token to a request on `/api`, `/auth` or `/live` delivers the secret to the API Worker and to anything that logs request headers there |
| BF3 (finding, passes 2-3) | case variants of the bypass prefixes, unauthenticated | INFO, **security-relevant** | `/API/ping`, `/Api/ping`, `/API`, `/AUTH/callback`, `/Auth/x`, `/LIVE/echo`, `/Live` all returned the **gated root app shell with no credentials**: Access's bypass destination match is **case-insensitive**, the Worker's routing is case-sensitive, so the request fell through to the static-asset SPA shell. Case variants of the *gated* targets are still gated (T1). See 13.4 |

### 13.4 What the platform did with the traversal / encoding / case probes (pass 3, 123 forms; each sent without and with the token)

| Edge behaviour | Probes | Unauthenticated | With token | Reading |
|---|---|---|---|---|
| **Dot-segments and `%2e` are normalised BEFORE Access matches**: `/api/../x`, `/api/%2e%2e/x`, `/api/%2E%2E/x`, `/api/.%2e/x`, `/api/%2e./x`, `/api/./../x`, `/api/x/../../x`, `/auth/../x`, `/live/%2e%2e/x`, `/./x`; also path-with-`?`, trailing `/`, `.`, `%20`, `;`, and upper-case forms of the gated targets, and `/API/../x`, `/Api/%2e%2e/x` | 59 | `302` to Access | gated content | Access evaluates the **normalised** path, so a bypass prefix cannot be used to climb into a gated route. 59 non-vacuous refusals |
| `..;`, `..%2f`, `%2e%2e%2f`, `..%5c`, `%252e%252e`, `%252e%252e%252f`, `%2e%2e;` under `/api/` or `/auth/` | 24 | `200` Worker JSON (`/api/...` echo) | same | **Not** treated as traversal: the path stays under the bypass prefix, Access bypasses it, the Worker sees an `/api/...` path and serves its API echo. No gated content |
| `//x`, `/api%2f..%2fx`, `/gated/asset-secret.txt%2e`, `;/api/x` suffixes | 15 | `302` Access | a non-Access redirect | refused unauthenticated; the authenticated-side redirect was not inspected (the one redirect I did inspect, below, is the assets service canonicalising a path) |
| `/internal/worker-secret/`, `/.`, `%20`, `;/api/x`, `%3b/api` | 7 | `302` Access | the Worker's `internal-other` 404 | gated (UI app), reach only a 404 |
| `%00`, leading `..` above root | 6 | `400` | `400` | rejected by the edge |
| `/api//../x` | 3 | **`302` Access** | **Worker `/api` echo** | **Access and the origin disagree** about this path: Access treats it as gated, the Worker serves `/api/...`. Safe direction (Access stricter). Only the `//..` form was tested, not a plain `/api//x` |
| case-variant bypass prefix + `..;` / `%2e%2e%2f` (`/AUTH/..;/x`, `/LIVE/%2e%2e%2fx`) | 5 (+1) | `307` to a normalised path (`/AUTH/..%3B/...`, `/LIVE/../...`) | `307` (and **one `522`** for `/AUTH/..;/` with the token, not reproduced or investigated) | bypass matched case-insensitively, then the assets service redirected to a canonical path; following the redirect re-enters the gate. No content |
| **case-variant bypass prefix, no traversal**: `/Api/`, `/Api/internal/worker-secret` (and from BF3 `/API/ping`, `/AUTH/callback`, `/LIVE/echo`) | 2 (+7 BF3) | **`200`, the root app shell, no credentials** | same | **LEAK of the gated UI shell** (1,712 bytes; no secret marker). Access matched the bypass app case-insensitively; the Worker did not match `/Api`; static assets fell back to `index.html` |
| `/Api/gated/asset-secret.txt` | 1 | **`403` (Access block page, 37 KB)** | gated content | Access matched the NESTED gated app (`/api/gated/*`) case-insensitively and applied the more specific policy: confirms O1's specificity rule and the case-insensitive match from the other side |

SNI/Host (H1, SP2): trailing-dot Host, upper-case Host, `:443`, `:8443`, trailing-dot Host+SNI and upper-case SNI all stayed gated. **When the TLS SNI and the Host header name different hostnames (either direction, between the product and space hostnames) Cloudflare answered `403`** — domain fronting between the two hostnames is not possible.

What to do with BF3 (recommendation, **[unverified]**: not deployed or re-tested): the bypass match is case-insensitive, so every case variant of `/api`, `/auth` and `/live` is public. Whatever the origin serves for such a variant is therefore public: the API Worker must (a) treat paths case-insensitively for these three prefixes or reject non-canonical-case paths with a 404/redirect **before** any static-asset fallback, and (b) never rely on the case-sensitive Access destination to keep a *gated* resource safe if that resource's path can be reached through a case variant of a bypass prefix. With `assets.not_found_handling = single-page-application` a missing path returns the root shell with 200, which is exactly what leaked; `run_worker_first` patterns are case-sensitive, so they cannot be enumerated to cover the variants (`run_worker_first: true` would, at the price of one billed invocation per asset request).

### 13.5 Latency on the real platform (A -> B service binding vs in-process), N = 1000 per kind, two independent passes

**Which is which.** *Laptop* = this Mac -> Cloudflare edge over one keep-alive HTTPS connection to `https://spike-0004.sow.foo/api/lat/<kind>` (a bypass path, so Access adds nothing); the four kinds are interleaved in rotating order per iteration, 100 warm-up iterations discarded, 0 errors, 0 reconnects. The numbers include the laptop-to-edge round trip, which is why `null` is about 16.6 ms. *In-Worker* = the edge Worker timing 500 sequential calls to Worker B inside one request with `Date.now()`; it only advances on I/O **[doc: security-model]**, so it cannot time the in-process kind and is indicative only.
`null` = edge Worker returns a constant; `inproc` = edge Worker runs `compute(20000)` itself; `binding` = edge calls Worker B with `env.B.fetch(Request)`; `rpc` = edge calls `env.B.compute(n)` (WorkerEntrypoint RPC). Same `compute()` in both Workers (identical hash checked in X1).

| kind (laptop, ms) | pass 1 p50 / p95 / p99 | pass 2 p50 / p95 / p99 |
|---|---|---|
| null (floor) | 16.65 / 22.92 / 46.32 | 16.55 / 24.31 / 46.83 |
| in-process | 16.60 / 24.42 / 42.18 | 16.68 / 24.67 / 49.00 |
| service binding, fetch | 18.87 / 27.61 / 47.70 | 19.62 / 29.47 / 60.46 |
| service binding, RPC | 18.76 / 25.93 / 51.11 | 18.85 / 28.08 / 50.86 |

| paired difference per iteration (laptop) | pass 1 median [95% bootstrap CI] | pass 2 median [95% CI] |
|---|---|---|
| binding(fetch) - in-process | **+2.42 ms** [+2.22, +2.72] | **+2.80 ms** [+2.48, +3.03] |
| binding(RPC) - in-process | **+2.26 ms** [+2.07, +2.52] | **+2.27 ms** [+2.07, +2.50] |
| in-process - null (the compute itself) | about 0 (p50 16.60 vs 16.65) | about 0 |

| in-Worker loop (indicative) | pass 1 | pass 2 |
|---|---|---|
| fetch binding, ms per call | 4.31 (runs 3.84-4.60) | 4.95 (runs 4.54-5.15) |
| RPC binding, ms per call | 3.91 (runs 3.75-4.12) | 3.76 (runs 3.70-4.61) |

**Reading.** Both independent passes show a binding hop of about **2.3-2.8 ms median** over the in-process path, with confidence intervals that exclude zero, and the minimum latency also shifts by about 2 ms (13.4-14.9 vs 11.3-12.5). That is roughly **+14% on a ~16.6 ms request** from this laptop, and it is larger than the +0.24/+0.38 ms measured locally under workerd. RPC and `fetch` differ by 0-0.5 ms, so most of the cost is the hop, not Request/Response/JSON construction. The in-Worker loop (3.8-5.0 ms per call) is higher than the laptop-paired delta and I cannot explain the gap (quantised `Date.now()` and the response parsing inside the loop are candidates), so treat it as supporting evidence that the hop is not free, not as a second precise measurement. p99s are dominated by the laptop network (about 46-60 ms for the floor itself).

**This does not confirm the documented "zero overhead or added latency" (§3.5).** It measures wall-clock time from outside; it cannot tell whether the cause is placement (the two Workers not sharing a thread in this account), isolate cold starts, or something else. Plan accounts for it as a real cost: **about 2-3 ms per service-binding hop on the hot path**, to be weighed against Option 1 (one Worker, no hop) in ADR-0090.

### 13.6 What differed from the documentation (or filled a gap in it)

| Topic | Docs said | Observed |
|---|---|---|
| Service-binding latency | "zero overhead or added latency", same thread | **+2.3-2.8 ms median per hop** (two passes, CIs exclude 0); local workerd +0.24-0.38 ms. Contradicts, or at least does not confirm, the claim on this account |
| Bypass destination matching | silent on case; says `/alpha/*` does not cover `/alpha` | **case-insensitive** (`/Api/x` bypassed; `/Api/gated/x` matched the nested gated app). `/api/*` does cover deep paths; `host/api` covers `/api/` |
| Path normalisation before matching | silent | the edge **normalises `..` and `%2e` / `%2E` before Access matches**; `..;`, `%2f`, `%5c`, `%252e` are not treated as traversal; `%00`/above-root `..` are `400` |
| Overlap | "more specific rule wins, nothing inherited" | confirmed (O1); an unauthenticated request to the service-auth-only nested app gets **`403`**, not the `302` login redirect the UI app gives (it has no identity policy to log in with); OPTIONS on a gated path is also `403` |
| Headers on a bypass path | silent | forged `Cf-Access-Jwt-Assertion` passes through, forged `Cf-Access-Authenticated-User-Email` is removed, `CF-Access-Client-Id`/`-Secret` pass through to the origin |
| Access + WebSocket | recommends hostname-based apps (Worker-level policies give 403) without saying they work | works: gated (service token) and bypass paths, text/binary/1 MiB, and through a service-binding hop. Soak: one 80 s drop in run 1 (ungated, cause unknown), two clean 120 s soaks in run 2 |
| SNI vs Host | silent | a mismatch (either direction between the two hostnames) is `403`; trailing-dot/upper-case Host and `:443`/`:8443` do not bypass |
| `workers_dev: false`, `preview_urls: false` | "hostname-based Access protects only that hostname; `workers.dev` bypasses it" | with both flags off, `workers.dev` and the version-prefixed preview URL return `404` for all three Workers |
| Custom Domain | "creates DNS and a certificate"; certificate is not deleted with the domain | consistent so far: the custom-domain resource carried a `cert_id` (edge `ab4348ad-1a13-425b-9ba1-05e5cc31dd6a`, space `6cacbde4-093a-4ef2-8219-92f0238ecccb`); the certificate-pack list is unreadable (403), so whether the destroy left those packs behind is **not verified** (§14). Custom-domain ids were identical in run 1 and run 2 (derived from the hostname) |
| Terraform provider v5 | `destinations = [{type = "public", uri = ...}]`, `decision = "non_identity"` for Service Auth | worked exactly as written in provider 5.28.0 (10 adds, 10 destroys, no drift on destroy) |
| `wrangler delete` with the API token | n/a | fails (needs KV read, §11); works with the owner's wrangler OAuth login when `CLOUDFLARE_ACCOUNT_ID` is set |
| Access policy list | n/a | run 1 (before the Access permission was added) listed 0 reusable policies with `success=true`; run 2 lists 7. Either an empty list was returned to a token without policy read, or the 7 were created in between; I cannot tell which. None are mine. Do not read an empty `access/policies` as "no policies" |

### 13.7 Propagation delays observed

- **Custom domain -> serving:** the first probe, about a minute after the create calls returned (1-2 s), already got Worker responses over TLS. **Not a certificate-issuance measurement**: the run-1 certificate packs were probably still in place (§11; packs are unreadable with this token, so unverified), which would explain TLS being ready immediately. A first-ever hostname would still need a certificate issued (unmeasured).
- **Access app -> enforcement:** at t+0 s after `apply` the gated path returned `302` to Access and the bypass path served the Worker; the nested overlap app was enforced (`403`) in the first checks pass about a minute later. No propagation delay observable at 1 s resolution; the 10 creates in the same `apply` took under 2 s.
- **Teardown -> gone:** the Access apps/policies/tokens and Workers were absent from the API immediately; the two DNS names returned no record from the authoritative nameservers right after teardown, but **this Mac's resolver cache kept answering `spike-0004.sow.foo` with 104.21.88.146 and 172.67.184.198 (curl then got Cloudflare `530`) for at least a minute after**; DNS-over-HTTPS and the authoritative servers returned nothing. Clients with a cached answer will see an error page, not the Worker, until their TTL expires.

## 14. Run 2 teardown proof (2026-10-18 13:59-14:01 UTC)

Command: `SPIKE_WRANGLER_AUTH=oauth ./run_platform.sh teardown` (hardened: every sub-step runs; deletes limited to the three fixed names). Status 1 only because the certificate step could not list packs.

| Sub-step | Result |
|---|---|
| `tofu destroy` | `[tested]` plan was exactly **0 to add, 0 to change, 10 to destroy**, all `spike-0004*`; `Destroy complete! Resources: 10 destroyed.` (2 custom domains, 4 Access apps, 3 policies, 1 service token) |
| `wrangler delete spike-0004-edge`, `-space`, `-b` (owner's OAuth login, `CLOUDFLARE_ACCOUNT_ID` = Woven, API token unset) | `[tested]` "Successfully deleted" x3, in that order (edge first because it binds to B) |
| `delete_cert_packs.py --yes` | `[tested]` `cannot list certificate packs (HTTPError): nothing deleted` — the token's certificate-pack read is 403, as expected. Nothing was attempted by another route |
| `verify-absent` | `[tested]` zero `spike-0004*` in **Workers scripts, custom domains, Workers routes, Access applications, Access policies, Access service tokens**; DNS records and certificate packs `UNVERIFIABLE` (403, allowed list, not counted as zero) |
| `compare` against the pre-apply snapshot + named baseline | `[tested]` **identical**: Workers 2 (`art-filestore`, `workflows-starter-template`), custom domains 2 (`secure.gallery`, `upload.secure.gallery`), Access apps 4 (`App Launcher`, `Warp Login App`, `Proton SSH (infrastructure)`, `ds-cleaner SSH (infrastructure)`), service tokens 3 (`foo`, `status-page-prober`, `warp-fleet-lambda`), reusable policies 7 (the 7 pre-existing ones listed in 13.1, none removed or changed), Workers routes 0 |
| DNS | `[tested]` the zone's authoritative nameserver (`ken.ns.cloudflare.com`) answers `NOERROR` with **no answer** for `spike-0004.sow.foo` and `spike-0004-space.sow.foo`, **identical to a never-created control name** (`random-nonexistent-9f3a.sow.foo`); DNS-over-HTTPS cannot resolve either. This Mac's OS resolver cache still returned two Cloudflare IPs for `spike-0004.sow.foo` (stale cache, `curl` -> `530`); `spike-0004-space.sow.foo` does not resolve at all. The API DNS inventory remains unreadable, so this authoritative check is the evidence |
| `scrub-state` | `[tested]` local OpenTofu state, plan and data dir removed (they held the service-token secret). Logs and reports under `<scratch>/spike-0004-out/` contain no secret (scanned for `cfast_`/`client_secret = "`) |

**Left behind (cannot be removed or listed with this token):**
- The **edge certificate packs** that the two custom domains created. Run 2's custom-domain resources carried `cert_id` **`ab4348ad-1a13-425b-9ba1-05e5cc31dd6a`** (`spike-0004.sow.foo`) and **`6cacbde4-093a-4ef2-8219-92f0238ecccb`** (`spike-0004-space.sow.foo`). The platform does not delete a custom domain's certificate with the domain **[doc]**; the pack list is 403, so whether these ids (or the run-1 packs, possibly the same ones) still exist is **unverified**. Owner action: dashboard **SSL/TLS > Edge Certificates**, delete any pack whose hosts are `spike-0004.sow.foo` / `spike-0004-space.sow.foo`; or add **SSL and Certificates Edit (and Read)** to the token and run `tests/delete_cert_packs.py --yes` (it only deletes a pack whose every host contains `spike-0004`).
- Nothing else. No Worker, custom domain, Access app, policy or service token named `spike-0004*` remains in any readable inventory.

**Token permissions found missing in this run:** SSL and Certificates read/Edit, DNS read, KV read (needed for `wrangler delete` with the token; the OAuth login was used instead). Held and exercised: Workers Scripts write (deploy), Custom Domains write, Access Apps and Policies write, Access Service Tokens write, Account Settings/Workers/Access read.

**Actual cost of run 2: about $0 [derived, not read from billing].** Roughly 11,000 requests to the Workers (latency 2 x about 4,400 + checks 3 x about 600 + a few hundred probes; WebSocket messages are not requests), well inside Workers Free's 100,000/day and the Paid plan's 10 M/month; static-asset requests are free; no interactive Access login occurred; no add-on was bought. Whether the Woven account is on Workers Free or Paid, and its Zero Trust plan, was not read.
