---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0011: Static asset hosting — Workers static assets (Cloudflare) / Caddy or nginx (self-hosted)

## Context and Problem Statement

Plane's `apps/web`, `apps/admin` (`/god-mode`), and `apps/space` (`/spaces`) are served by a Caddy proxy that also routes `/api`, `/auth`, `/live` [S] (`apps/proxy`). **Corrected in round 26: `web` and `admin` are static SPAs (`ssr: false`) but `space` is server-rendered (`ssr: true`), see the Update below.** On Cloudflare something must serve those files and route paths to Workers. This is one of the ports listed in ADR-0002 (`StaticHost`).

## Decision Drivers

* Remove Caddy on the Cloudflare adapter set.
* Same-origin routing of UI, API and collab paths (cookies, CSRF, WebSocket).
* Keep the self-hosted twin working with the existing proxy.

## Considered Options

* Option 1: Workers static assets bound to the edge Worker (CF); Caddy/nginx (self-hosted)
* Option 2: Cloudflare Pages per app
* Option 3: Serve assets from R2 via a Worker
* Option 4: Keep Caddy on a VM/container for both sets

## Decision Outcome

Chosen option: **Option 1**, because Workers static assets give same-origin routing with the API Worker and need no extra service, while the self-hosted set keeps its existing proxy. Source: interview 2026-10-17, round 3 (Workers static assets selected as decided).

### Consequences

* Good, because one deployment unit, path routing in the Worker, and no proxy to run.
* Bad, because two apps need separate build configs: `web` at `/` on the product hostname and `space` built for its own public hostname (`apps/admin` is retired, ADR-0021).
* Neutral: SPA fallback and cache headers are configured per app.

### Confirmation

Smoke test per adapter set: both apps (web, space) load, deep links resolve to the right app shell, and `/api`, `/auth` and `/live` still route to their Workers on the same origin.

## Pros and Cons of the Options

### Option 1: Workers static assets

* Good, because same origin as the API; atomic deploy of UI + API.
* Bad, because asset limits and prefix handling need checking.

### Option 2: Pages per app

* Good, because simple per app.
* Bad, because multiple origins complicate cookies/CORS and routing.

### Option 3: R2 via Worker

* Good, because flexible.
* Bad, because it reinvents static hosting, with worse caching.

### Option 4: Keep Caddy

* Good, because zero change.
* Bad, because contradicts the Cloudflare-native goal.

## Update — review decisions, round 25 (2026-10-18)

Host layout (round 25): **one hostname per product surface, same-origin UI + `/api` + `/auth` + `/live`** so cookies, CSRF and the collab WebSocket stay simple. Cloudflare Access is not used at P0 (user+password). At the CF Access phase it is applied with **path-scoped Access applications: UI paths gated; `/api`, `/auth` and `/live` bypass Access** because the application authenticates them itself (session, `X-Api-Key`, signed collab token). Access + WebSocket-upgrade behaviour is verified in spike 0004. The retired admin app means only two static apps (web, `space` on its own public hostname) are served.

## Update — review fixes, round 25 (2026-10-18)

Consequences and Confirmation restated for two apps (web, `space`) and one same-origin hostname per surface, following ADR-0021 and the round-25 host layout above. Substance unchanged.

## Update — spike results, round 26 (2026-10-18)

**Correction (found by the spike 0009 inventory and checked against `apps/*/react-router.config.ts` in both the pre-pin and the pinned trees):** `apps/web` and `apps/admin` are SPAs (`ssr: false`), but **`apps/space` is `ssr: true`**, with a server loader that fetches the API (`apps/space/app/issues/[anchor]/layout.tsx` in the pre-pin tree). It cannot be served as plain static assets. Workers static assets therefore hold for `web` only; the public `space` Worker (ADR-0022) needs an SSR runtime or an SPA build. That is a new owner question (Q-0011.2).

**Q-0011.1 answered (spike 0004: documentation, local runs and one real-edge run `[tested-run1]`):** there is no shared asset namespace to manage — `admin` is retired and `space` has its own hostname and Worker — so each Worker serves one SPA at `/` with `not_found_handling = "single-page-application"`. Two rules are mandatory. (1) `assets.run_worker_first` must list **both forms** of every Worker-handled path (`/api`, `/api/*`, `/auth`, `/auth/*`, `/live`, `/live/*`), because a browser navigation to `/auth/callback` (the OIDC return from Zitadel) is otherwise answered with `index.html` and never reaches the Worker; this held on the real platform. (2) The SPA fallback is one per Worker and always the root `/index.html`, and a missing hashed asset is answered `200 text/html`, not 404, so deploy with version affinity and keep old hashes until clients age out. Putting both apps in one Worker under `/` and `/spaces/` would need a Worker handler for the second shell and a base-path build of `space`; whether Plane's `space` builds with a base path is unverified, and the host layout makes it unnecessary.

## Update — owner decisions and spike results, round 27 (2026-10-18)

The case-insensitive Access bypass (ADR-0050, Q-0050.7) interacts with `not_found_handling = "single-page-application"`: a path such as `/Api/x` is bypassed by Access, is not matched by the case-sensitive `run_worker_first` patterns, and is answered with the root app shell and a 200. On the real platform (spike 0004 run 2) the same checks confirmed the other rules above: navigations to `/auth/callback`, `/auth` and `/api` reached the Worker only because of `run_worker_first`, the gated shell and static assets stayed gated, and `/spaces/` served its own shell.

## Update — owner decision, round 27 (2026-10-18)

Owner decision on Q-0011.2: **host `space` as an SSR Worker.** Build the React Router SSR output with the Cloudflare Vite plugin (a supported, generally available path `[doc]`: the scaffold is `ssr: true`, the plugin runs server code in the Workers runtime), on its own hostname and Worker as before (ADR-0022). This is a **bounded non-Rust exception** under the ADR-0006 round-22 policy: it serves the existing `space` app, holds no business logic, and fetches data through the API (a service binding or the public API URL; to settle in the feasibility build). **Exit plan:** the Dioxus public viewer replaces it in Phase 2 (ADR-0013). **First step:** a feasibility build of `apps/space` under workerd of about one day (bead `plane-byd.7`; result in `research/0009-space-ssr-worker.md`) — Node-only APIs in the server bundle are the main risk, and the Cloudflare Vite plugin does not support React Router's SPA mode or prerendering `[doc]`, so the fallbacks are an SPA build served as plain assets or the Node server behind the tunnel (ADR-0094).

## Update — space SSR feasibility result, round 27 (2026-10-18)

**Feasibility build of `apps/space` as an SSR Worker (`research/0009-space-ssr-worker.md`, bead `plane-byd.7`): feasible with listed changes, no blocking item.** Everything ran locally (workerd 1.20261012.1, wrangler 4.151.1, `@cloudflare/vite-plugin` 1.64.0, headless Chrome); nothing was deployed and the API was a stub. `[tested]` below means local.

* **It renders.** The otherwise unmodified app builds and renders under workerd; Chromium hydrates with no page errors and no hydration warnings.
* **What SSR actually buys is link previews only.** The one server loader (`apps/space/app/issues/[anchor]/layout.tsx`, checked by the coordinator in the pinned tree) fetches `/api/public/anchor/{anchor}/meta/` and the route's `meta` function turns it into the title, description and og/twitter tags; the board itself is rendered on the client (`useSWR`). So the server-rendered HTML holds the per-board `<head>` and a spinner, with no issue titles when JavaScript is off. The Node build behaves the same.
* **Changes needed.** Config only: `@cloudflare/vite-plugin` and `wrangler` added, `allowBuilds: workerd: true` in `pnpm-workspace.yaml`, the plugin in `vite.config.ts`, `future.unstable_enableNodeReadableStream: true` in `react-router.config.ts`, and new `wrangler.jsonc`, `public/_headers` and `workers/app.ts` (73 lines). `nodejs_compat` is not needed. **No app code must change.** Recommended: set `initAsync: false` in `packages/i18n/src/core/instance.ts` (tested; replaces a `setTimeout` shim) and add a fetch timeout to the loader (a 6 s API delay currently gives a 6 s SSR response).
* **Build gotchas.** Build with the root base path (`/spaces` breaks static assets on Workers) and an absolute `VITE_API_BASE_URL` (the Docker default of `""` makes the loader throw `Invalid URL`). Node-only items: the default Node server entry is replaced by the web-stream entry through the future flag; `process.env` is inlined at build; i18next's module-scope `setTimeout` is shimmed or fixed by `initAsync: false`; `vfile`'s `node:` imports resolve natively at compat date 2026-10-12.
* **Numbers (local).** Bundle 13,995 KiB uncompressed (limit 64 MiB), 3,080 KiB gzip; 616 of 618 modules are locale chunks. Startup 5 to 8 ms active. SSR p50 2.0 ms, p95 3.0 ms, workerd CPU about 0.65 ms (the Node control: p50 0.9 ms, CPU 1.2 to 1.6 ms), N=300.
* **Caching.** The response carries no `Cache-Control` and no `Vary`; the HTML depends only on the anchor. The entry must set an explicit `Cache-Control` and rely on Workers Caching; with it enabled and no header a 200 gets a 2 h heuristic TTL. A Cache API wrapper was tested locally (miss, then hit, with no API call); Workers Caching itself is not emulated locally, so it is unverified. Unknown boards return 200, so keep the TTL short and purge on publish and unpublish.
* **Data path.** The Worker can call the API by public URL or by service binding (proven with an unresolvable API hostname). The browser still calls the API itself, so it needs CORS with credentials or a same-origin allow-list proxy (Q-0011.4).
* **Effort** to productionise: about 8.5 person-days (range 6 to 12).
* **ADR-0006 exception record:** a JS Worker that serves static assets and the per-board `<head>` metadata from one public read-only API call; it never holds business logic, auth decisions, cookies or writes; exit plan is the Dioxus viewer (Phase 2).
* **Not proven:** anything on Cloudflare's network, the real API, concurrency, the new flag on the Node image, non-Chromium browsers.

**An alternative the finding opens up** `[inferred, not built]`: because only the `<head>` needs the server, `space` could be built as a plain SPA and the **Rust API Worker** could inject the per-board metadata into the static `index.html` (it already serves the data behind `/api/public/anchor/{anchor}/meta/`). That removes the JavaScript exception altogether, at the cost of a small edit to the one server loader (outside the Q-R2 list) and a Rust handler. Q-0011.3 asks which to take; the SSR Worker stays the standing decision meanwhile.

## Update — space hosting decision (Q-0011.3, supersedes the SSR Worker), round 27 (2026-10-18)

Owner decision on Q-0011.3 (**supersedes the "SSR Worker" decision of Q-0011.2 above**): the feasibility build showed that SSR produces only the per-board `<head>`, so `space` is built as a **plain SPA** and the **Rust side injects the per-board metadata** into the static `index.html`. There is no JavaScript exception for `space`; the SSR Worker build (`research/0009`, config-only, about 8.5 pd) stays documented as the fallback.

Design direction (coordinator; confirm at planning): (1) the one server loader in `apps/space/app/issues/[anchor]/layout.tsx` becomes a `clientLoader`, so the browser still gets the tab title; the app builds with `ssr: false` and the root base path. (2) A small **Rust** handler on the `space` hostname serves the static assets and, for `/issues/{anchor}`, reads `index.html` from the assets binding, fetches `/api/public/anchor/{anchor}/meta/` through the API (service binding), and inserts the title, description and og/twitter tags before `</head>`. (3) **Every inserted value is HTML-escaped** (the board name, description and cover URL are user-controlled; ADR-0141). (4) Short explicit `Cache-Control`, keyed on the anchor, purged on publish and unpublish; decide whether an unknown anchor returns 404 or Plane's 200 with default metadata. (5) Whether this is a route group in the main Worker bound to a second custom domain or its own thin Worker is a planning-time choice (ADR-0022 says a separate cached Worker). Not yet checked: that `space` builds cleanly with `ssr: false` after the loader change (about half a day; bead `plane-byd.8`). The effort is unmeasured, probably below the SSR Worker's because there is no JavaScript Worker build or deploy pipeline `[inferred]`. Q-0011.4 (CORS or a same-origin proxy) still applies.

## Update — review round 29 (2026-10-19)

Per FR-19 and FR-11 (`reviews/2026-10-19-fable-architecture-review.md`) and an owner decision of round 29 (owner, round 29), **the self-hosted native binary serves the built React apps itself**, as Workers static assets do on the Cloudflare set. A front proxy (Caddy or nginx) is the operator's optional choice, not part of the twin. This keeps ADR-0091's Compose file with at most three services: the binary, Postgres, and MinIO from P1 (ADR-0091, round 29). It supersedes the self-hosted half of this ADR: "Caddy or nginx (self-hosted)" in the title, "Caddy/nginx (self-hosted)" in Option 1, the third Decision Driver ("Keep the self-hosted twin working with the existing proxy"), and the Decision Outcome's "while the self-hosted set keeps its existing proxy". The title is kept as history. Neither the binary (`crates/thrust-server/src/main.rs`) nor the Worker (`crates/thrust-worker/wrangler.toml`) serves the static apps yet.

**`space` hosting is settled.** The `space` app keeps its own public hostname (ADR-0022), and that hostname is routed to the one Worker (FR-11; ADR-0090, round 29), which serves the `space` SPA, the Rust head-injection handler and the API on that same hostname. The browser therefore calls the API same-origin, so neither CORS nor a proxy is needed: Q-0011.4 is answered with FR-11's default (review §4, "`space` as an SPA with Rust head injection"). This settles item (5) of the round-27 design direction above (a route group in the main Worker, not its own thin Worker) and replaces its closing "Q-0011.4 still applies". The rest of that design direction stands, except that item (2)'s metadata fetch "through the API (service binding)" becomes a call inside the same Worker (ADR-0090's round-29 note removes the `SELF` service binding). One constraint follows from the round-26 rules above and is not settled here: a Worker has one static-assets directory and one SPA fallback (the root `/index.html`), so `web` and `space`, both built at the root base path, cannot share them as they stand. How the one Worker keeps the two shells and their hashed assets apart on the two hostnames is an implementation point; it does not change the decision.

## Open Questions

* Q-0090 — Worker topology and routing across static, API and collab. (answered round 29, FR-11)

## More Information

Related: [[ADR-0002]], [[ADR-0090]]. Paths: `apps/proxy` (Caddyfile.ce), `apps/web`, `apps/admin`, `apps/space`.
