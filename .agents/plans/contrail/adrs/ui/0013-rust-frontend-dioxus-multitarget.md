---
status: accepted
date: 2026-10-18
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0013: Rust frontend rewrite (Dioxus) for multi-target builds

## Context and Problem Statement

ADR-0001 keeps Plane's React apps and rewrites only the backend. In round 21 the owner set the long-term direction: the
backend is Rust from the start (ADR-0006), the React frontend is adopted with minimal changes, and **once backend and
React frontend are stable the UI is rewritten in Rust with Dioxus as a clone of the Plane UI/UX**, diverging from Plane's
UX only after frontend and backend are both stable in Rust. The purpose is **multi-target compilation** — web app,
desktop, mobile and others — from one codebase.

## Decision Drivers

* One language and shared types across backend and clients.
* Multi-target delivery (web, desktop, mobile) without separate codebases.
* Keep delivery risk low: ship value on the proven React UI first.
* Cloudflare-native hosting of the web target.

## Considered Options

* Option 1: Keep React permanently (web only) — ADR-0001 as originally written
* Option 2: React now, Dioxus clone later, then diverge (owner direction)
* Option 3: Dioxus from the start (no React phase)
* Option 4: React + Tauri/Capacitor wrappers for desktop/mobile

## Decision Outcome

Chosen: **Option 2**, as recorded by the owner in round 21. Phase 1 uses the React apps with minimal changes (service
layer, auth, collab provider only — scope to confirm, Q-R2). Phase 2 begins when backend + React frontend are stable
(definition of "stable" to set, Q-R3). Divergence from Plane's UX happens only after the Dioxus app reaches parity.

### Consequences

* Good: shared Rust types end to end; one UI codebase for all targets; Cloudflare hosts the web build as static assets.
* Bad: ~195k LOC of apps (web/admin/space; ~175k in `apps/web`) plus ~100k LOC of shared packages including the design system must be re-implemented; the rich-text editor (TipTap/ProseMirror) has no mature
  Rust equivalent (JS interop or webview likely); accessibility, i18n and virtualised/drag-drop components are younger in
  Dioxus; the React app is transitional so investment in its integration layer is partly throwaway (Q-R1).
* Neutral: Q-G8 ("web apps only") is superseded in the long term; until Phase 2 only web applies.

### Confirmation

Spike 0007 (early feasibility, no-go triggers listed there) and a go/no-go note before Phase 2 starts. A no-go keeps React
and drops multi-target (reopen this ADR). See `research/0002-rust-dioxus-fit.md`.

## Pros and Cons of the Options

### Option 1: Keep React
* Good: lowest cost. Bad: no multi-target; two languages permanently.

### Option 2: React now, Dioxus later
* Good: ships fast, de-risks the backend first, one rewrite at a stable moment. Bad: two UI efforts over time.

### Option 3: Dioxus from the start
* Good: no throwaway integration. Bad: delays any usable product; high risk with the editor.

### Option 4: React + wrappers
* Good: cheap desktop/mobile. Bad: not Rust, not what the owner wants.

## Update — interview round 22 (2026-10-18)

Gate and targets: Phase 2 starts when **P0+P1 tiers are shipped, the 99.5%/p95 < 500 ms SLO has held for 4 weeks, and the v1 oracle diff is clean** (Q-R3, Q-0013.1). **Thrust v1 targets are web only; desktop and mobile follow after v1** (Q-0013.3). Minimal React changes are limited to base URL/service layer, auth client, collab provider and branding (Q-R2). Non-Rust exceptions: see ADR-0006.

## Update — spike 0006 results, round 27 (2026-10-18)

Input from spike 0006 (`research/0008-editor-conversions.md`, section 10): the **Yjs binary stays the long-term source of truth** for any collaboratively edited document; JSON and HTML are projections (JSON is exactly derivable from the binary, the reverse loses history and client identity, HTML loses more). The ProseMirror-JSON model in `thrust-editor-doc` (compiles for wasm32 and native) is the natural shared intermediate representation for a Dioxus editor, so the web, desktop and mobile targets share one schema crate. Dioxus should **not** parse HTML as an input format: HTML import is a lossy, quirk-ridden ingest path that belongs on the server. What Rust lacks is the ProseMirror *view* layer (selection, input rules, IME, drag handles): that is Q-0013.2, not a data-model gap. Keep the schema versioned: the client's `UniqueID` block ids already make the client and headless schemas differ, and a Dioxus client would add a third unless the schema crate is the single definition that generates all of them.

## Open Questions

* Q-0013.1 — Definition of "stable" that triggers Phase 2.
* Q-0013.2 — Rendering approach per target (WASM DOM, WGPU renderer, webview) and the rich-text editor strategy.
* Q-0013.3 — Which targets are required in v1 (Thrust): web only, plus desktop, plus mobile?
* Q-0013.4 — Does the Dioxus app share generated API types with the backend crates, or an OpenAPI-generated client?

## More Information

Related: [[ADR-0001]], [[ADR-0006]], [[ADR-0010]], [[ADR-0011]], [[ADR-0012]]. Spikes: 0003, 0006, 0007.
