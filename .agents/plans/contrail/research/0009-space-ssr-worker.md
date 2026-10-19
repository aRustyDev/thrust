# Research 0009 — Feasibility build: `apps/space` as a server-rendered Cloudflare Worker

Date: 2026-10-18 · Bead `plane-byd.7` · Decision Q-0011.2 · ADR-0011 (round 27), ADR-0022, ADR-0006 (non-Rust exception policy)
Status: **one-day feasibility build done. Verdict: FEASIBLE WITH LISTED CHANGES. No blocking item found. Nothing was deployed; every number is local.**

Evidence tags (each claim carries one):

| Tag | Meaning |
|---|---|
| **[tested]** | I ran it, locally: workerd 1.20261012.1 via `vite preview` / `wrangler dev --local`, and headless Chrome for Testing 147. Never Cloudflare's network |
| **[doc]** | stated on a Cloudflare page or in the pinned source (path given) |
| **[inferred]** | derived from tagged facts; assumptions stated |
| **[unverified]** | not established; listed in section 9 |

Everything below was built in a scratch copy of the pinned tree (`scratchpad/space-ssr/`, section 10). The pinned tree was only read. The API is a **stub** (`stub-api.mjs`), written from `apps/api/plane/space/{urls,views,serializer}`; its shapes are not Plane's real responses.

## 0. Read this first

1. **It runs.** The unmodified `apps/space` app builds with `@cloudflare/vite-plugin` and renders under workerd: the server loader fetches the (stub) API, the HTML is streamed, a real Chromium hydrates it, and the published board appears (8 issues in list view; 200 virtualised rows for the 600-issue board in kanban). Behaviour matched the Node build on every check I ran (section 6). **[tested]**
2. **No app logic had to change.** Everything needed is build/deploy config plus one Worker entry file (`workers/app.ts`, 73 lines). Two small hardening changes to app code are recommended, not required (A1, A2 in section 3). **[tested]**
3. **What SSR actually delivers is only the `<head>`.** The one server `loader` fetches the board's `meta`. The body that the server renders is the spinner: `InstanceProvider` shows `LogoSpinner` until a client-side SWR call returns instance info. With JavaScript disabled, `body` text is empty and the HTML holds **0** issue titles, on the Node build and on the Worker alike. So the step-3(a) check "issue titles in the server HTML" **fails for both builds, by app design**. What the Worker does deliver server-side is the per-board `<title>`, description, Open Graph and Twitter tags (link previews). **[tested]**
4. **Four things broke on the way and each has a fix** (section 4): module-scope `setTimeout` in `@plane/i18n` (workerd forbids it), the default Node `entry.server` (`renderToPipeableStream`), React Router 8's stricter load context, and `workerd` blocked by pnpm's build-script policy. A fifth is a deployment trap: a `/spaces` base path breaks static assets on Workers. **[tested]**
5. **Bundle 13.7 MiB uncompressed (21 % of the 64 MiB limit), startup 5-8 ms active, about 0.65-0.73 ms workerd CPU and p50 2.0-2.1 ms per SSR request locally.** **[tested, local]**
6. **Data path.** The browser, not the Worker, makes nearly all API calls, with a build-time API origin. The Worker's own call is one `fetch` for meta. Both public-URL and service-binding paths work in workerd (binding proven with an unresolvable API hostname). Both need an **absolute** origin at build time; the Docker default (`VITE_API_BASE_URL=""`) makes the loader throw `Invalid URL` and every board gets the default title. **[tested]**
7. **Caching.** The SSR response carries no `Cache-Control` and no `Vary`. Public boards are cacheable by path (the HTML does not depend on cookies, language or user agent, **[inferred]** from the code). I verified a Cache API wrapper locally (MISS then HIT, zero API calls) and wrote the Workers Caching form (`cache.enabled` plus explicit `Cache-Control`), which local dev does not emulate. **[tested] / [unverified on the platform]**
8. **Effort to productionise: about 8.5 person-days (range 6-12)**, section 7.

## 1. Environment

| Item | Version |
|---|---|
| Pinned tree | `.../agent-a69ad036a9c59b0a8` (read-only), `apps/space` 1.4.2: React Router 8.3.0, React 19.2.8, Vite 8.0.16, i18next 25.10.9 |
| Added | `@cloudflare/vite-plugin` 1.64.0, `wrangler` 4.151.1 (workerd 1.20261012.1, so `compatibility_date` is capped at 2026-10-12) |
| Toolchain | pnpm 11.10.0, Node v26.9.0 on this Mac (the repo pins 22.22.0 and the Dockerfile uses `node:22-alpine`, so the Node **control** ran on a newer Node than production) |
| Browser | Playwright (Python) 1.64.0 driving the cached Chrome for Testing 147.0.7727.15 headless shell |
| Stub | `stub-api.mjs`: boards `small-board` (8 issues), `large-board` (600 issues, 20 KB description), `boom` (500), anything else 404; CORS with credentials; per-request log |

## 2. Inventory of the server side of `apps/space` (step 1)

| Area | Finding |
|---|---|
| `react-router.config.ts` | `ssr: true`, `basename` from `VITE_SPACE_BASE_PATH` (Docker default `/spaces`), `future.unstable_optimizeDeps` |
| `vite.config.ts` | `base` = same path; `define: {"process.env": JSON.stringify(viteEnv)}` bakes every `VITE_*` var **at build time**; alias `next/navigation` to a shim; `dotenv`, `node:path` (config only) |
| `entry.server.*` | **none in the app.** `@react-router/dev` picks its Node default (`entry.server.node.tsx`: `node:stream` `PassThrough`, `renderToPipeableStream`) because `@react-router/node` is a dependency (`dist/typegen-*.js:461`). `entry.client.tsx` awaits `@plane/i18n` `initPromise`, then `hydrateRoot` |
| Routes | `/` (auth view; `headers` export `X-Frame-Options`), `/:workspaceSlug/:projectId` (**`clientLoader` only**: redirects to `/issues/<anchor>`), layout `issues/[anchor]/layout.tsx` + `issues/:anchor`, catch-all `*` (not-found component, **HTTP 200**) |
| Server `loader` | **one**: `issues/[anchor]/layout.tsx`. Validates `anchor` against `/^[a-zA-Z0-9_-]+$/`, then `fetch(\`${process.env.VITE_API_BASE_URL}/api/public/anchor/${anchor}/meta/\`)`. Non-2xx or any throw returns `{metadata: null}` (default title "Plane"). No timeout. No `action` anywhere |
| `headers` | root: `Referrer-Policy`, `X-Content-Type-Options`, `X-DNS-Prefetch-Control`, `Strict-Transport-Security`; index route: `X-Frame-Options`. No `Cache-Control` anywhere |
| `process.env` / env | only `VITE_*` (build-time). `VITE_API_BASE_URL` is read by the loader and by `@plane/constants` (`API_BASE_URL`, used by every `@plane/services` class in the **browser**). No runtime env, no secrets |
| Node-only code in the SSR graph | `@react-router/node` default entry; `vfile` imports `node:path`, `node:process`, `node:url` (three lines at the top of `server-build.js`); `i18next` init at module scope. No `fs`, `crypto`, `stream` (outside the default entry), `Buffer` use on the tested paths; axios is bundled but unused on the server (the loader uses `fetch`) |
| Module-global state | `lib/store-provider.tsx` builds one `RootStore` at module scope and returns it for every server render (the "always create a new store" comment is not what the code does). The server never populates it (no effects run), so no cross-request leak today; it would leak in Node and in a long-lived isolate alike **[inferred]** |
| How it runs today | `Dockerfile.space`: `turbo prune`, `pnpm turbo run build --filter=space`, runner `node:22-alpine`, `react-router-serve ./build/server/index.js`, healthcheck `GET /spaces/`; Caddy (`apps/proxy`) routes `/spaces` to it. `nginx/nginx.conf` is a leftover from the SPA era (static root plus `try_files ... /spaces/index.html`) |
| Node build size (control) | `build/server` 12 MB in 616 files (`index.js` 5.4 MB), `build/client` 15 MB |

## 3. What I changed (step 2)

All paths are under the pinned-tree layout. Diff: `space-ssr-worker.patch` (171 lines, 7 files) and `space-ssr-worker-lockfile.patch` (510 lines), section 10.

| # | File | Change | Class |
|---|---|---|---|
| 1 | `apps/space/package.json` | devDependencies `@cloudflare/vite-plugin` 1.64.0, `wrangler` ^4.151.1 | build config |
| 2 | `pnpm-lock.yaml` | regenerated | build config |
| 3 | `pnpm-workspace.yaml` | `allowBuilds: workerd: true`. Without it `pnpm install` exits 1 with `ERR_PNPM_IGNORED_BUILDS` (hit twice, also inside `pnpm exec`'s dependency check). **[tested]** | build config |
| 4 | `apps/space/vite.config.ts` | import `cloudflare`, add `cloudflare({ viteEnvironment: { name: "ssr" } })` first in `plugins` | build config |
| 5 | `apps/space/react-router.config.ts` | `future.unstable_enableNodeReadableStream: true`: selects the web-stream default entry (`renderToReadableStream`). `v8_viteEnvironmentApi` no longer exists in RR 8 (always on) | build config |
| 6 | `apps/space/wrangler.jsonc` (new) | name, `main: ./workers/app.ts`, `compatibility_date: 2026-10-12`, **no** `nodejs_compat`, `workers_dev: false`, `preview_urls: false`, `assets.binding: ASSETS`, `vars.SSR_CACHE_TTL`, `cache.enabled: true`, `observability`; commented `services` example | deploy config |
| 7 | `apps/space/workers/app.ts` (new) | Worker entry: `createRequestHandler` over `virtual:react-router/server-build`; load context as a `RouterContextProvider`; `setTimeout` shim (see 4.1); optional `API` service-binding `fetch` shim; explicit `Cache-Control` | build/deploy glue (no business logic) |
| 8 | `apps/space/public/_headers` (new) | `/assets/*  Cache-Control: public, max-age=31536000, immutable` (served by Workers static assets; confirmed locally) | deploy config |
| 9 | `apps/space/worker-configuration.d.ts` | **generated** by `wrangler types` (626 KB; not in the patch). With it and `workers/app.ts` in the program, `react-router typegen && tsc --noEmit` exits 0 (I confirmed with `--listFilesOnly` that both files are checked) | build config |
| 10 | build environment | `VITE_SPACE_BASE_PATH=""` (root), `VITE_API_BASE_URL` = absolute API origin | env |

**App-code changes: none required.** Recommended, in priority order:

| # | File | Change | Evidence |
|---|---|---|---|
| A1 | `packages/i18n/src/core/instance.ts` | add `initAsync: false` to `init()`. Removes the need for the `setTimeout` shim. Shared package, so web and admin need a regression run | **[tested]** Worker and browser both pass with the shim deleted |
| A2 | `apps/space/app/issues/[anchor]/layout.tsx` | give the loader `fetch` a timeout (`signal: AbortSignal.timeout(n)`) | **[tested]** a 6 s API delay gives a 6.0 s SSR response on Node and on the Worker; there is no bound |
| A3 | same loader | optional: take the API fetcher from the load context (service binding) instead of the `fetch` shim in the entry; about 4 lines | **[tested]** the shim form works; the loader form is **[inferred]** |
| A4 | same layout | optional: a route `headers` export for `Cache-Control` instead of setting it in the entry | **[inferred]** |
| A5 | same loader | optional behaviour change: unknown anchor returns a real 404 instead of a 200 soft 404 (today both runtimes answer 200) | **[tested]** current behaviour |

## 4. What broke on the way (and the fix)

Build: the first `react-router build` with the plugin **compiled with no errors** (about 25 s). Every failure below was at run time.

| # | Symptom (first request) | Cause | Fix |
|---|---|---|---|
| 4.1 | 500 `Disallowed operation called within global scope. Asynchronous I/O ... setting a timeout ... not allowed within global scope` | `@plane/i18n` runs `i18n.init()` at module scope; i18next 25 schedules `setTimeout(load, 0)`. Found by wrapping `setTimeout`: stack `I18n.init` at `server-build` | Entry: while the server build evaluates, zero-delay timers run as microtasks (no app change). Clean fix: A1. **Both tested** |
| 4.2 | 500 `Invalid context value provided to handleRequest ... must return an instance of RouterContextProvider` | the docs' `{ cloudflare: { env, ctx } }` form is rejected by RR 8 | `new RouterContextProvider()` + `createContext`; no loader uses it yet |
| 4.3 | 500 `renderToPipeableStream is not a function` | Node default entry selected because `@react-router/node` is installed; the edge build of `react-dom/server` has no pipeable stream | future flag (change 5) |
| 4.4 | `pnpm install` exit 1, `ERR_PNPM_IGNORED_BUILDS workerd` | pnpm 11 `allowBuilds` policy | change 3. The binary runs without its postinstall (`workerd --version`), so `false` would also work; `true` lets the validator run |
| 4.5 | with `VITE_SPACE_BASE_PATH=/spaces`: `/spaces/assets/<hash>.js` returns `200 text/html` | client output is `build/client/assets/...`; Workers static assets match by path under the assets root, so `/spaces/assets/...` misses and falls to the React Router catch-all | build for the root (own hostname, as ADR-0011 already assumes). A prefix would need the client output nested under `build/client/spaces/` **[doc]** (research 0006 s3.6). **[tested]** both builds; root works |

## 5. Run under workerd (step 3)

Servers: Worker through `vite preview` (18788) and through plain `wrangler dev --local -c build/server/wrangler.json` (18791); `react-router dev` (Vite dev server with the plugin) also served the page (first request 4.2 s of on-demand transforms, correct title). Node control: `react-router-serve` on the unmodified build (18787, `/spaces`). **[tested]**

**(a) Server-rendered HTML.** Worker and Node return the same `<head>`: `<title>Stub Small Board</title>`, description, `og:*`, `twitter:*` (the loader's server-side `fetch` is visible in the stub log: user-agent `node` vs empty for workerd). Issue titles: 0 occurrences in both. Chromium with JavaScript off: `body` text `""`, title and `og:title` correct. Diff of the normalised HTML (both built at `/spaces`): same structure; differences are asset hashes, the un-minified `next-themes` inline script, and the `unstable_enableNodeReadableStream` flag in the router context.

**(b) Hydration, real browser (Chrome for Testing 147).** `/issues/small-board/`: board visible after hydration, 8 rows, **no page errors, no hydration warnings**; `/issues/large-board/?board=kanban`: 200 virtualised rows, no page errors; deep link `/stub-ws/<project id>/` redirects (client loader) to `/issues/small-board/?board=list` and renders. Console errors in every run, **identical on Node**: `401 /api/users/me/` (anonymous viewer, expected) and `404 /auth/get-csrf-token/` (my stub has no such route). Screenshot: `out/worker-small.png`.

**(c) Assets and base path.** Root base: hashed assets served by Workers static assets with `cache-control: public, max-age=31536000, immutable` (from `_headers`); missing assets and unknown paths answer `200 text/html` from the catch-all, **same as the Node build** inside its basename. `/spaces` base: breaks, section 4.5.

**(d) Headers and edge caching.**

| Response | Headers observed (Worker, `wrangler dev`) |
|---|---|
| Board document | `content-type: text/html` (Node adds `; charset=utf-8`; the document starts with `<meta charSet>`), the four root security headers, chunked; **no `Cache-Control`, no `Vary`** (`Vary: Origin` appears only under `vite preview`; Vite adds it). Node: no `Cache-Control` either |
| What the HTML depends on | the anchor only. The server never reads cookies, `Accept-Language` or the user agent (language is the fallback `en`; `isbot` only changes whether the stream waits). `?board=` and `?peekId=` do not change the render **[inferred]** from the code. No `Vary` is needed |

What would need to change to cache public boards:

1. Send an explicit `Cache-Control` on the board document. Without one, a Worker response is not cached by the CDN; with Workers Caching enabled and none set, a 200 gets a **2 hour heuristic TTL** **[doc]** `workers/cache/configuration`, which would also cache the "board not found" shell (HTTP 200) for 2 hours. My entry sets `public, max-age=0, s-maxage=<SSR_CACHE_TTL>, stale-while-revalidate=<SSR_CACHE_TTL>` for `GET /issues/<anchor>` and `no-store` for everything else. **[tested]** (headers) **[unverified]** (platform behaviour).
2. Pick the mechanism. Cloudflare now recommends **Workers Caching** (`cache.enabled`, wrangler >= 4.69.0): runs in front of the Worker, honours `Cache-Control`, collapses concurrent requests, tiered, purge by tag or path; the host is not in the cache key **[doc]**. Local dev does **not** emulate it: with `cache.enabled` and TTL 60, every request still hit the stub. The Cache API is the alternative: **[tested]** a wrapper (`alt-workers-app.cacheapi.ts`) gave MISS then HIT with zero API calls (stub log 2005 to 2006 over 3 requests; p50 1.57 ms vs 2.43 ms, workerd CPU 0.70 vs 0.78 ms per request). It is per data center, not tiered, does not collapse bursts, and is unavailable behind Cloudflare Access **[doc]** `workers/runtime-apis/cache`.
3. Choose a short TTL (30-60 s fits ADR-0022's 60 req/min/IP) and purge on publish and unpublish. Because unknown anchors answer 200 with the default title, a cached miss would hide a board published a moment later, and a cached hit keeps an unpublished board's title and description visible until expiry. Option A5 (real 404) makes this cleaner.
4. Rate limiting: a service-binding or same-account `fetch` reaches the API without the end user's IP, so the 60 req/min/IP rule (ADR-0033) cannot be enforced on the SSR leg at the API edge; the cache plus a rule on the space hostname must carry it **[inferred]**.

## 6. Measurements and controls (steps 4 and 5)

All **[tested, local]**: one MacBook, workerd 1.20261012.1, loopback, sequential keep-alive requests, N = 300 per row after 30 warm-up requests; CPU = user+system time of the server process tree sampled around the run. These are not Cloudflare numbers.

| Quantity | Result |
|---|---|
| Bundle (`wrangler deploy --dry-run`) | 618 modules, **13,994.76 KiB** uncompressed; Total Upload 14,036-14,038 KiB; **gzip 3,080 KiB**; limit 64 MiB uncompressed (no compressed limit) **[doc]** `workers/platform/limits`. Composition: `server-build.js` 7.5 MiB (the app and its deps, not minified), **616 of the 618 modules are i18n locale chunks (22 languages x 28 namespaces, about 6.0 MiB)** **[tested]**. Client assets: 691 files, 15 MB |
| `wrangler check startup` | window 104-146 ms, **active 5.1-7.6 ms**, 2 samples. This covers global scope only: the server build is imported lazily on the first request |
| First request after process start | 125-192 ms wall (module load, i18n init, render); workerd CPU about 3.4 ms plus 2.3 ms in the wrangler parent. Most of the wall time is waiting, probably miniflare's module fallback, so it is **not** a prediction of Cloudflare cold start **[inferred]** |
| SSR per request, `wrangler dev` (final build) | small board p50 **2.00** / p95 3.01 / p99 5.38 ms, workerd CPU 0.65 ms; large board p50 2.13 / p95 3.50 / p99 5.56 ms, CPU 0.73 ms; unknown board p50 1.80 / p95 3.04 ms, CPU 0.61 ms |
| Same, `vite preview` (extra proxy hop) | p50 2.0 / p95 3.4 ms, workerd CPU 2.1 ms (plus 0.8 ms Node proxy) |
| Node control (`react-router-serve`, Node 26.9) | p50 0.93 / p95 1.89 ms, CPU 1.65 ms (small); p50 0.83 / p95 1.93 ms, CPU 1.15 ms (large) |

"Large board" is the 20 KB description (HTML 89 KB vs 9 KB): the SSR cost does **not** depend on the number of issues because the server never fetches them; only the description size matters. The server HTML for the 600-issue board has the same size as for any board with that description.

Controls (each shows a check can fail, or isolates the cause):

| Control | Result |
|---|---|
| Unknown board, browser | `board_visible=false`, client 404s on `settings`/`states`/`labels`; server answers **200** with `<title>Plane</title>`. **Node does the same** (curl: 200, same title) |
| Stub returns 500 (`boom`) | server 200 with default title; browser `board_visible=false`; same on Node |
| API down (stub killed) | server 200, default title, loader logs `Error fetching issue metadata: ... Network connection lost` (Node: `ECONNREFUSED`) |
| API slow (6 s) | SSR takes 6.0 s on both runtimes: no timeout (A2) |
| Invalid anchor (`bad%20anchor`) | regex rejects it, no API call, 200 default title; Node same |
| Without the `setTimeout` shim | 500, global-scope error (4.1); with the shim, or with A1 and no shim, 200 |
| `nodejs_compat` needed? | A probe Worker with **no flags** at compat date 2026-10-12 imported `node:path`, `node:process`, `node:url` and used `Buffer`: all worked. The app runs with `compatibility_flags: []`. So not needed on the paths tested; whether the platform's deploy-time module validation agrees is **[unverified]** |
| Service binding | build with `VITE_API_BASE_URL=http://api.stub.invalid` (does not resolve) plus an `API` binding to a stub Worker: `<title>Stub Small Board VIA SERVICE BINDING</title>`; unknown anchor via the binding gives the default title |
| Relative API base (`VITE_API_BASE_URL=""`, the Dockerfile default) | loader `TypeError: Invalid URL: /api/public/anchor/small-board/meta/`; default title for every board; no request reaches the API. By the same code Node fails the same way **[inferred]** |

## 7. Verdict (step 6)

**Feasible with listed changes.** The changes are config (items 1-10) plus the Worker entry; no change to app logic. Blocking items: none found. The two real risks are product-level, not technical:

- **The value of SSR here is link previews only** (finding 3). First paint is a spinner in both builds. If that is all the owner wants from SSR, the fallback already named in ADR-0011 (SPA build as plain assets) plus a Worker that injects the per-board `<head>` (HTMLRewriter) would preserve it with fewer moving parts **[inferred]**; I did not build or estimate that alternative. Productionising the SSR route is estimated below at 8.5 person-days and carries one JS exception. Q-0011.2 stays the owner's call.
- **Time and unbounded waits**: the loader has no timeout (A2) and unknown boards answer 200 (A5); both matter once caching and the 60 req/min/IP rule are in place.

### Node-only dependencies and what replaced them

| Dependency or API | Where | Fate |
|---|---|---|
| `entry.server.node` (`node:stream`, `renderToPipeableStream`, `@react-router/node`) | RR default entry | replaced by the web-stream default via `unstable_enableNodeReadableStream`; `@react-router/node`, `@react-router/serve`, express stay as dependencies for the Node image only |
| `process.env.*` | loader, `@plane/constants` | no polyfill: Vite `define` inlines all `VITE_*` at build time (0 `process.env` left in the server bundle) |
| `node:path`, `node:process`, `node:url` | `vfile` (markdown path) | resolved by workerd natively at this compat date **[tested]**; `nodejs_compat` not enabled |
| `setTimeout` at module scope | `i18next` init in `@plane/i18n` | entry shim or A1 |
| `fs`, `crypto`, `Buffer`, `stream` | none on the tested paths | n/a |
| `axios` | `@plane/services` | bundled, never called on the server |
| `dotenv`, `node:path` | `vite.config.ts` | build time only |
| cookies, sanitisers, mobx | none on the server path; mobx renders fine; the shared module-level `RootStore` is unchanged behaviour | n/a |

### Production data path

- **Browser leg (unchanged by hosting):** `@plane/services` calls `${VITE_API_BASE_URL}/api/public/anchor/<a>/{settings,states,labels,issues,...}/` and `/api/instances/`, `/api/users/me/` with `withCredentials: true`. Either (a) **cross-origin**: absolute API origin, the API must return `Access-Control-Allow-Origin: <space origin>` with credentials (so no `*`), or (b) **same-origin**: empty base, the space Worker must route `/api/*` to the API, which means `run_worker_first: ["/api", "/api/*"]` and a proxy handler restricted to the public read-only paths. (b) is new Worker code and must stay allow-list only. Today's Docker default is (b) without the proxy, and breaks the loader (section 6).
- **Server leg, option A, public API URL:** build with the absolute origin; zero code; one subrequest per document; the API sees the Worker's egress, not the user. Needs the API host to be reachable from Workers and not Access-gated.
- **Server leg, option B, service binding:** add `"services": [{"binding": "API", "service": "<api-worker>"}]`; the entry's shim (or A3) sends the loader's fetch over it. **[tested]** with a stub API Worker (`wrangler dev -c a -c b`, documented as experimental in research 0006 s3.5). No public hop; billed as one request; CPU is the sum across both Workers **[doc]** research 0006 s3.5. The browser leg still needs (a) or (b).

### ADR-0006 exception record (dated note for ADR-0011 / ADR-0006)

> 2026-10-18 — `space` SSR Worker (plane-byd.7). A JavaScript Worker (React Router 8 server build, Cloudflare Vite plugin, `workers/app.ts`) that serves the static client bundle and renders the document shell and per-board `<head>` metadata from one public read-only API call (`/api/public/anchor/<a>/meta/`). It must never hold business logic, make authentication or authorisation decisions, read or forward cookies or credentials, write data, or call anything outside the public read-only surface. Exit: the Dioxus public viewer (ADR-0013, Phase 2); earlier exit if SSR is judged to add nothing beyond link previews (replace with SPA plus Worker metadata injection).

### Effort to productionise (person-days)

| Work | Days |
|---|---|
| CI build: `allowBuilds`, committed lockfile, `wrangler types`, split `tsconfig` for `workers/`, per-environment `VITE_*` plumbing, bundle shrink (minify SSR, ship only needed locales) | 1.5 |
| Worker deploy config in the IaC: custom domain, `workers_dev`/`preview_urls` off, `cache`, observability, service binding | 1.0 |
| Data path: binding or public URL, loader timeout (A2), browser leg (CORS or `/api` allow-list proxy) | 2.0 |
| Caching: `Cache-Control` policy, TTL, purge on publish and unpublish, soft-404 decision (A5) | 1.5 |
| Tests: CI smoke on workerd, Playwright hydration check, error-path assertions, cache assertions | 1.5 |
| Platform verification: real deploy, headers, cold start, Access bypass hostname, `node:*` acceptance, Workers Caching | 1.0 |
| Total | **8.5 (6-12)** |

## 8. Reproduce

Scripts live in `scratchpad/space-ssr/` (section 10). Order: `copy.sh` (pristine `orig/` and `work/` copies), `pnpm install` in `work/`, `build-node.sh` (control), `build-worker.sh ""`, `run-stub.sh`, `run-wrangler-dev.sh <ttl>`, `probe.sh`, `browser-check.py`, `measure.py`, `make-patch.sh`.

## 9. Not proven

- Anything on Cloudflare's network: deploy acceptance (`node:*` modules without `nodejs_compat`), cold start, real CPU and latency, Workers Caching hits, purge, Access bypass on the public hostname.
- Real Plane API shapes (stub only), real CORS and cookie behaviour, the `/auth/get-csrf-token/` call.
- Concurrency: all load was sequential. Isolate memory (128 MB) and the shared `RootStore` under concurrent requests were not exercised.
- `unstable_enableNodeReadableStream: true` on the **Node** build: not run. If the self-hosted image must keep building, gate the flag on an environment variable.
- Free-plan viability (10 ms CPU per request **[doc]**): local CPU is far below it, but production CPU is unmeasured.
- Only Chromium was used (no Firefox or WebKit); only the list and kanban layouts; no calendar, gantt, spreadsheet, peek overview or sign-in flows.
- `react-router dev` was checked for one request, not for HMR.

## 10. Artifacts (scratch, not in the plan package)

Directory: `/private/tmp/claude-502/-Users-asmith-repos-woven-forks-plane--claude-worktrees-docs-plane-on-cloudflare-spec/5dd6cc30-19bc-4362-9294-2aac5bb1243c/scratchpad/space-ssr/`

- `space-ssr-worker.patch` (unified diff, `orig/` vs `work/`, 7 files), `space-ssr-worker-lockfile.patch`
- `alt-workers-app.cacheapi.ts` (the tested Cache API variant of the entry)
- `stub-api.mjs`, `stub-api-worker/`, `probe-node/` (node-module control), `browser-check.py`, `measure.py`, `probe.sh`
- `out/`: browser JSON per run, `measure-*.jsonl`, `probe-*.log`, `worker-small.png`, `space-startup*.cpuprofile`, build logs
