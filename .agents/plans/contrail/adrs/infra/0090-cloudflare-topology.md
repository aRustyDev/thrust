---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0090: Cloudflare Worker topology and domain layout

## Context and Problem Statement

The Cloudflare adapter set (ADR-0002) must host an API surface (internal API, public `/api/v1`, `/auth`), queue/cron
consumers, a collab Durable Object class, and the two kept React apps (`web`, a static SPA, and `space`, which is server-rendered, so its hosting is Q-0011.2; the admin app is retired, ADR-0021) (ADR-0011). Today Caddy routes
`/api,/auth`→Django :3004, `/live`→:3005, `/spaces`→:3002, `/god-mode`→static admin, everything else→static web **[S]**.
We must decide how many Workers exist, how they talk, and how hostnames map. Platform facts (verified against CF docs,
2026-10): Workers on the same zone can only call each other via **service bindings** (or via a Custom Domain);
service bindings add no latency/cost; a Worker using a binding requires the target to be deployed first; during gradual
deployments two Workers can sit at different versions (**version skew**); Durable Object class lifecycle changes are
atomic and cannot be rolled out gradually or rolled back across.

## Decision Drivers

* Both adapter sets share one application codebase (ADR-0002); topology must be a *composition* concern only.
* DO lifecycle changes should not ride along with ordinary API deploys (blast radius).
* Gradual deployment + rollback of the API must be possible without touching the collab DO class.
* Keep `.woven` private-hostname rules intact where private access is needed (see Open Questions).
* Minimise cross-Worker hops on the hot request path.

## Considered Options

* Option 1: Single Worker (router + consumers + DO class + static assets in one deployment)
* Option 2: Role-split Workers — `edge` (static assets + routing), `api`, `consumer` (queues/cron), `collab` (DO class) — joined by service bindings
* Option 3: Domain-split Workers (workspace, work-items, pages, notifications… one Worker each) via service bindings

## Decision Outcome

Decision pending — see Q-0090. Leading hypothesis: **Option 2** (confidence: medium). It isolates the DO lifecycle in
`collab`, lets `api` use gradual deployments, and keeps the hot path to one hop (edge→api) while avoiding the
version-skew surface of Option 3.

### Consequences

* Good, because DO class changes deploy independently; `api` rolls forward/back freely.
* Good, because `consumer` can scale/redeploy without affecting request latency.
* Bad, because inter-Worker contracts (RPC entrypoints) need versioning discipline: deploy callee first, keep compatible.
* Neutral: self-hosted twin maps roles to processes (api server, worker process, hocuspocus) — see ADR-0091.

### Confirmation

A topology smoke test per environment: wrangler config lint asserting (a) only `collab` declares DO `exports/migrations`,
(b) each binding target exists, (c) no `fetch()` same-zone Worker-to-Worker calls (must be bindings). CI job in ADR-0100.

## Pros and Cons of the Options

### Option 1: Single Worker

* Good, because simplest deploy, zero skew, one config.
* Bad, because DO lifecycle changes are coupled to every API release; gradual deployment is unavailable when `exports` lifecycle changes ship in the same version.
* Bad, because bundle size/startup grows with all domains and consumers.

### Option 2: Role-split Workers

* Good, because blast-radius isolation by operational role; matches how the self-hosted twin splits processes.
* Good, because static assets stay on `edge` with content-hashed files; use version affinity during rollouts to avoid asset 404 skew.
* Bad, because 3–4 deployables and ordered deploys (callee first).

### Option 3: Domain-split Workers

* Good, because team-level independent deploys.
* Bad, because many service-binding edges and skew combinations; repository layer/transactions get awkward across Workers.
* Bad, because disproportionate for single-org-first scope.

## Update — interview round 5 (2026-10-17)

Q-0090.1 answered: **both audiences** — product hostnames are **public Cloudflare hostnames behind Access**; **admin/internal hostnames are private `.woven` names**. Consequences: (a) the woven two-leg rule (Cloudflare hostname route + CoreDNS rewrite) applies to the `.woven` hosts, and per the woven hostname rules a new name on woven-o11y is created by annotating a Service — **there is no equivalent for a Worker**, so the Workers-side reachability mechanism (tunnel origin pointing at a Worker custom domain, or another path) must be designed and verified (new **Q-0090.3**); (b) topology must be host-agnostic per audience: the admin app (`/god-mode`) and any internal API surface may live on a separate Worker route/host from the product; (c) any `.woven` host must assert *resolves and serves on the right port*, not an IP or prefix, and must not be Access-gated if probed by `status.woven`; (d) Access is therefore part of the P1 auth step for the public product hosts.

## Update — interview round 9 (2026-10-17)

Cost envelope set (Q-G5): **Workers Paid baseline, under $100/month** as a design constraint. Spikes (ADR-0040 and any DO/Queues/Workflows spike) must report projected monthly cost at the sizing target (≤50 users, ≤100k work items).

## Update — interview round 13 (2026-10-17)

Private hosts (Q-0090.3): owner direction is a **tunnel origin to a Worker custom domain**, keeping `.woven` names for the CF set. Mechanism and Access interaction need a topology spike. Recovery targets: RPO 24 h, RTO 8 h.

## Update — review decisions, round 25 (2026-10-18)

Host layout (round 25): **one hostname per product surface, same-origin UI + `/api` + `/auth` + `/live`** so cookies, CSRF and the collab WebSocket stay simple. Cloudflare Access is not used at P0 (user+password). At the CF Access phase it is applied with **path-scoped Access applications: UI paths gated; `/api`, `/auth` and `/live` bypass Access** because the application authenticates them itself (session, `X-Api-Key`, signed collab token). Access + WebSocket-upgrade behaviour is verified in spike 0004. The retired admin app means only two static apps (web, `space` on its own public hostname) are served.

## Update — review fixes, round 25 (2026-10-18)

**Durable Object class placement:** only the `collab` Worker declares DO exports/migrations. Sessions are DB-backed (round 25), so there is **no session DO**; a rate-limiter DO (ADR-0033 Option 2, not chosen) would also have to live in `collab`. There is no `/god-mode` app. Q-0050.2 is answered (path-scoped Access bypass for `/api`, `/auth`, `/live`). ADR stays `proposed` pending spike 0004.

## Update — spike results, round 26 (2026-10-18)

**Spike 0004, public half** (`research/0006-topology-public-half.md`): Cloudflare documentation, local workerd runs and one real-Cloudflare run **without Access** (run 1). The run with Access applications was halted once by a missing token permission and then completed; see the round-27 update below. Nothing found contradicts the leading hypothesis (Option 2); confidence stays **medium** until the Rust Worker is re-measured.

| | Option 1: single Worker | Option 2: role-split + bindings | Option 3: domain-split |
|---|---|---|---|
| Access path scoping | works | works | works. Access keys on hostname + path, independent of how many Workers sit behind it `[doc]` |
| Hop cost | none | docs: zero overhead, no extra request fee, CPU summed `[doc]`; local: +0.24 ms (RPC) and +0.38 ms (fetch) median over 1,000 paired calls `[measured-local]`, not production numbers; **real edge (run 2): +2.3 to +2.8 ms per hop**, so the "zero overhead" claim is not reproduced | same per hop, more hops |
| WebSocket across a hop | n/a | edge → Worker B works on the real edge, 256 KiB binary intact `[tested-run1]`; edge → DO untested | untested |
| Public exposure | one `workers.dev` to disable | every Worker needs `workers_dev = false` and `preview_urls = false` `[doc]`; a pure back-end Worker has no public surface | N Workers to harden |

* **Durable Object placement (spike 0001, `[tested]`):** DO classes must be exported from the same Wasm module as `fetch`, so a DO cannot live in a separately built crate of the same Worker. The collab DO therefore needs its **own Worker root** (a second composition-root crate depending on a `thrust-collab-do` library) or a re-export by `thrust-worker`; the first fits Option 2. The spike's rate-limiter DO was exported from `thrust-worker`.
* **Q-0090.2 answered (desk): Custom Domains** for both product hostnames. The Worker is the origin, DNS and the certificate are created automatically, and `cloudflare_workers_custom_domain` is one resource; a Route is only for a hostname that keeps another origin. Caveats `[doc]`: exact hostname only, no existing CNAME, `Workers Routes Write` on the zone, and **deleting a Custom Domain leaves its edge certificate behind** (the first run's packs could not be listed or removed with the available token, so whether they remain is unverified).
* **Q-0030.2 answered (desk): one `api` Worker with one auth stack** for the compat-plane routes, the core API and `/api/v1` (route groups inside one router; the credential choice is middleware). Access cannot tell the surfaces apart and protects neither, and service bindings add no latency or fee, so a split would not be slower; the argument for one Worker is deployables and the transitional `compat-plane`. Revisit if the combined Rust Worker breaches 64 MiB or 1 s (the spike 0001 slice measured 1.58 MiB and 17.7 ms) or if third-party API traffic gets its own hostname.
* **Real-edge observations from run 1 (no Access):** custom-domain TLS answered at the first probe (propagation under about two minutes, not timed more finely); `workers.dev` returned 404 for the edge Worker; service-binding fetch, RPC and in-process calls returned the same result; a browser navigation to `/auth/callback` reached the Worker only because of `run_worker_first`.
* **`.woven` half (Q-0090.3): a desk recipe only; nothing was built.** A private `.woven` name cannot be a bare pass-through to a Worker Custom Domain: the WARP path is port-preserving L4, and the Worker answers only its own public hostname and certificate. The recipe is an annotated K8s proxy Service in woven-o11y (so external-dns creates both legs) that terminates TLS for `x.woven` with an org-CA certificate and re-originates to the Worker's public hostname with that hostname as SNI and Host, presenting an Access service token. Alternative B is a public hostname whose Access policy requires the Cloudflare One client. Workers VPC is the wrong direction (Worker → private network). Four owner decisions come before anything is built: the K8s proxy Service or alternative B; where the service-token Secret lives (ADR-0092); which hostname counts as "internal" now that `admin` is retired (ADR-0021); and an infra-repo root for the proxy.
* **Not established:** the Rust Worker re-run and the `.woven` half. The Access behaviours and the service-binding latency were measured in run 2 (round-27 update).

## Update — owner decisions and spike results, round 27 (2026-10-18)

**Spike 0004 run 2 (real platform, Access gate on; `research/0006-topology-public-half.md` sections 13 and 14).** The run completed; the Access findings are in ADR-0050 (one security finding: the bypass match is case-insensitive, Q-0050.7). Topology-relevant results:

* **A service-binding hop costs about +2.3 to +2.8 ms (fetch) and +2.3 ms (RPC) median on the real edge** — two passes of 1,000 paired calls from a laptop, confidence intervals exclude zero, about +14 % on a ~16.6 ms request. The documented "zero overhead or added latency" is **not reproduced** (local workerd showed +0.24 to +0.38 ms); the cause is not isolated. Plan 2–3 ms per hop on the hot path: it favours few hops on the request path (edge → api, with the collab Worker the only extra hop) and is immaterial against the 500 ms p95 SLO at one or two hops. Option 2 stays the leading hypothesis, confidence medium.
* A more specific Access application wins and nothing is inherited (overlap test O1). With `workers_dev` and `preview_urls` off, `workers.dev` and the version-prefixed preview URL answer 404 for all three Workers. The public `space` hostname answers without credentials and serves none of the product host's gated content.
* Custom domains answered over TLS about a minute after creation (possibly reusing the run-1 certificates, so this is not a clean issuance measurement); Access applications were enforced at the first check; the Terraform provider (5.28.0) resources worked as documented (10 created, 10 destroyed). Teardown was clean except for two edge certificate packs that the token cannot list or delete.
* `wrangler delete` needs KV read with an API token; with the token alone it fails, while the OAuth login works (CI will need a token with KV read).

## Update — owner decision, round 27 (2026-10-18)

The public `space` Worker is a **JavaScript SSR Worker** (ADR-0011, round 27), not a Rust Worker: a second Worker type in the topology, built with the Cloudflare Vite plugin, bound to the API like the others. This fits Option 2 (role-split Workers) and leaves the Rust API Worker unchanged.

## Update — space hosting decision (Q-0011.3, supersedes the SSR Worker), round 27 (2026-10-18)

The public `space` hostname is served by **Rust**, not a JavaScript SSR Worker (Q-0011.3, which supersedes the round-27 note above): a small handler that serves the static SPA and injects the per-board metadata; whether it is a route group of the main Worker or its own thin Worker is open for planning. The topology has no JavaScript Worker.

## Open Questions

* Q-0090 — topology choice.
* Q-0090.1 — Is a private `.woven` hostname required for any Worker-served host? Per the woven hostname rules a `.woven` name needs **two legs** (CF hostname route + CoreDNS rewrite) and on woven-o11y is created by annotating a Service; a Worker on a public Custom Domain behind Access needs neither. Decide public-Access vs WARP-private before choosing hostnames.
* Q-0090.2 — Custom Domains vs Routes per host (Custom Domain points all paths; needs exact hostname).
* Q-0050.2 — Access scope (UI only vs UI+API).

## More Information

CF docs: service bindings, custom domains, gradual deployments (version affinity/overrides), DO migrations/`exports`.
Related: [[ADR-0002]], [[ADR-0011]], [[ADR-0060]], [[ADR-0091]], [[ADR-0110]].
