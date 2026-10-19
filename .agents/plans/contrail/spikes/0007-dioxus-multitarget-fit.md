# Spike 0007 — Dioxus: multi-target UI and Cloudflare fit (early feasibility)

**Gates:** ADR-0013 (Rust UI rewrite), ADR-0011 (static hosting), ADR-0012.
**Questions:** Q-G8 (clients), Q-0013.1–.4 (see QUESTIONS.md).
**Timebox:** 4 working days; **run early, small** — the full rewrite starts only when backend + React frontend are stable.
**Depends on:** spikes 0001, 0003, 0006. **Owner:** TBD.

## Question
Can Dioxus deliver a Plane-UI clone that builds for web, desktop and mobile from one codebase, hosted on Cloudflare,
and which parts of the Plane UI are hard (rich-text editor, drag-and-drop boards, virtualised lists, i18n, a11y)?

## Method
1. Hello-world fullstack app: web build served by Workers static assets (SPA) vs SSR through a Worker
   (`dioxus-cloudflare`-style bridge); compare bundle size, first load, hydration, cost.
2. Same crate builds desktop and (if feasible in timebox) one mobile target; record per-target gaps.
3. Pick three hard Plane screens and prototype: (a) issue list with grouping + virtualisation at 10k rows, (b) kanban
   drag-and-drop, (c) rich-text editor surface — embed `@plane/editor` through JS interop (web) / webview (desktop+mobile)
   vs a native Rust editor. Record interop cost.
4. Shared types: the API client and domain types come from the same Rust crates as the backend (OpenAPI-generated or
   direct crate sharing); prove one type change breaks both at compile time.
5. Auth from the client (cookies on web; bearer/token on desktop/mobile) using the spike 0003 result.
6. i18n (Plane ships many locales), accessibility (keyboard + screen reader on the three screens), theming parity.
7. Size the clone: component inventory of `packages/ui` + `apps/web` routes to estimate rewrite effort.

## Exit criteria
Go/no-go note with measured bundle sizes, a list of "needs JS interop" surfaces, and an effort estimate for the clone.
**No-go triggers:** rich-text editing cannot be hosted acceptably in non-web targets; virtualised list at 10k rows
janks; a11y gaps with no mitigation. A no-go keeps the React frontend and drops multi-target (reopen ADR-0013).

## Deliverables
ADR-0013 confirmation section; effort estimate; Q-G8 and Q-0013.1–.4 updated.
