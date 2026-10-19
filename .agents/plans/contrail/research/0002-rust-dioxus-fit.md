# Research 0002 — Rust backend, Dioxus frontend, BetterAuth, and the Cloudflare-native goal

Date: 2026-10-18 (interview round 21). Method: web search + fetches of vendor docs/READMEs; **nothing here was run**.
Confidence tags: **[doc]** stated by a vendor/README page I read · **[snippet]** only seen in a search snippet ·
**[unverified]** my inference or not found. Every [snippet]/[unverified] item is a spike exit criterion (see `spikes/`).

## 1. Direction as understood (owner, round 21)
1. Backend is **Rust from the start**; the existing Plane React frontend is adopted with minimal changes.
2. When backend and React frontend are stable, rewrite the frontend in **Rust (Dioxus)** as a **clone of the Plane UI/UX**;
   diverge from Plane's UX only after frontend and backend are both stable in Rust.
3. The rewrite's purpose is **multi-target compilation** (web app, desktop, mobile, …).
4. The new repo is **`git@github.com:aRustyDev/thrust.git`** (v1 name Thrust; MVP name Contrail).

## 2. Q: Can we use BetterAuth + Dioxus?
**Yes, with two caveats.** BetterAuth is a server-side auth library; Dioxus is a client UI framework, so they never
need to share a runtime.
- **Server:** BetterAuth itself is TypeScript. A community Rust port exists, `better-auth-rs` — **not official, alpha**
  (`1.0.0-alpha.3`), targets route/payload/cookie wire compatibility with TS better-auth 1.7.x, with email/password,
  cookie sessions (bearer opt-in), API keys, generic OAuth/OIDC, organizations, 2FA/passkeys; SeaORM database adapter;
  MIT/Apache-2.0 **[doc]**. Whether it builds for `wasm32`/Workers is **not documented [unverified]**. Alternatives:
  run TS BetterAuth as a service-bound identity Worker, or hand-roll a Rust `Identity` adapter (the fallback already
  recorded in ADR-0050). → Spike 0003.
- **Client:** BetterAuth's TS client does not run in Dioxus, but its protocol is plain HTTP JSON + cookies, so a Rust
  client (reqwest/gloo-net) can call it. Web Dioxus builds can use same-origin cookies; **desktop/mobile builds lack a
  browser cookie jar**, so they need the bearer-token flow (the port supports bearer via explicit config **[doc]**).
- The existing React app keeps using the TS BetterAuth client unchanged during phase 1.

## 3. Q: How well does Dioxus fit the Cloudflare-native goal?
**Good for hosting the web target; the cross-platform promise is the part with Cloudflare-independent risk.**
- Dioxus 0.7 targets web (WASM), desktop, iOS/Android and fullstack/SSR; has a WGPU-based HTML/CSS renderer alongside
  webview targets **[doc]**.
- Cloudflare hosting paths: (a) **SPA/WASM as Workers static assets** — straightforward, no Dioxus-specific
  support needed **[unverified but standard]**; (b) **SSR/server functions on Workers** via the community crate
  `dioxus-cloudflare` ("bridge between Dioxus server functions and Cloudflare Workers"; D1/KV access) **[snippet — could not
  fetch its docs; maturity, maintainer and Dioxus-version compatibility unknown]**. Plan on (a) first.
- The backend choice reinforces the fit: workers-rs lists bindings for KV, R2, D1, Queues (feature flag), Durable
  Objects, Service Bindings, Workers AI, Hyperdrive, Analytics Engine, Secrets Store, Rate Limiting, and events `fetch`,
  `scheduled`, `queue` **[doc: developers.cloudflare.com/workers/languages/rust]**. The workers-rs README marks Queues
  beta and D1 alpha, says panics abort the instance by default (unwind needs nightly), and Tokio/threads are unsupported
  **[doc]**. Workflows, Email Service, and WebSocket hibernation support were **not mentioned [unverified]**.
- Hyperdrive lists rust-postgres 0.19.8 as a supported driver **[doc]**.
- Limits that matter for Rust WASM: 64 MiB uncompressed Worker size, 1 s startup **[doc]**.
- `yrs` (Rust Yjs port) supports Yjs update formats v1/v2, so a Rust Durable Object can interoperate with the existing
  JS editor **[snippet]**.
- **Biggest Dioxus risks (not Cloudflare-specific):** (1) a rich-text editor equivalent to `@plane/editor`
  (TipTap/ProseMirror) — no mature Rust equivalent known; likely JS interop or webview; (2) rewriting ~195k LOC of UI
  plus the design system; (3) accessibility, i18n and virtualised/drag-drop components in a younger ecosystem;
  (4) multi-target auth and storage differences. → Spike 0007 (and 0006 for the editor data model).

## 4. Consequences found while reviewing the ADRs/QUESTIONS
See `QUESTIONS.md` Q-R1–Q-R6 and the round-21 notes in the affected ADRs. Headline: **the React frontend is now
transitional**, which weakens the case for a redesigned internal API plus a regenerated TypeScript client (ADR-0010 /
ADR-0030 were decided when the React app was permanent). That is a decision for the owner (Q-R1).

## Sources
- Cloudflare Workers Rust docs: https://developers.cloudflare.com/workers/languages/rust/
- workers-rs: https://github.com/cloudflare/workers-rs
- Hyperdrive + Postgres drivers: https://developers.cloudflare.com/hyperdrive/configuration/connect-to-postgres
- Workers limits: https://developers.cloudflare.com/workers/platform/limits/
- better-auth-rs: https://github.com/better-auth-rs/better-auth-rs
- Dioxus 0.7: https://dioxuslabs.com/learn/0.7/
- dioxus-cloudflare (search result only): https://crates.io/crates/dioxus-cloudflare
- yrs: https://docs.rs/crate/yrs/latest
