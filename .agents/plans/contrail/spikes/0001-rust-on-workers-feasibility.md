# Spike 0001 — Rust on Workers feasibility and dual-target vertical slice

**Gates:** ADR-0006 (runtime/language), ADR-0002 (ports), ADR-0007 (compile-time bundles), ADR-0071 (workflows), ADR-0073.
**Questions:** Q-0006, Q-0006.1, Q-0007.1, Q-0044.1, Q-0070.1.
**Timebox:** 5 working days. **Depends on:** none. **Owner:** TBD.

## Question
Can the backend be written once in Rust and built both as a `wasm32` Cloudflare Worker (workers-rs) and as a native
server (axum/tokio) with identical behaviour, using ports as traits and cargo features as compile-time adapter sets?

## Hypothesis
Yes for HTTP, KV, R2, Queues, Durable Objects, Hyperdrive and cron; gaps are likely in Workflows, Email and anything
that assumes threads/Tokio. Gaps are bridgeable with a thin JS shim Worker if they are narrow.

## Method
1. Cargo workspace with `ports` (traits), `core` (use-cases), `adapters-cloudflare`, `adapters-selfhosted`, two
   composition roots (`worker`, `server`).
2. Vertical slice: **create work item** = authz check → sequence allocation → insert → outbox row → queue publish →
   consumer writes activity. Plus `GET` list with soft-delete visibility.
3. Build both targets; run the **same contract-test suite** against both (workerd via `wrangler dev`/Miniflare for wasm,
   testcontainers for native).
4. Binding coverage matrix — for each of: KV, R2, Queues (producer **and** consumer), Durable Objects (incl. SQLite
   storage, WebSocket hibernation), Hyperdrive, Cron (`scheduled`), Workflows, Email Service, AI Gateway, Rate Limiting,
   Secrets Store, service bindings: record `native in workers-rs` / `via wasm-bindgen/JS shim` / `REST fallback` / `not possible`.
5. Measure: uncompressed bundle size (limit 64 MiB), startup time (limit 1 s), cold-start latency, p95 of the slice,
   behaviour on panic (`panic=abort` vs unwind).
6. Async/runtime: confirm no Tokio dependency leaks into `core`; record the executor abstraction used.

## Exit criteria
- **Pass:** slice green on both targets with one shared suite; bundle < 20 MiB and startup < 300 ms; every binding needed
  at P0 (KV, R2, Queues, DO, Hyperdrive, Cron) is native or a thin shim; panic handling policy defined.
- **Conditional pass:** Workflows/Email require a shim Worker — record the shim contract and where it lives.
- **Fail:** any P0 binding is unusable from Rust, or the dual build forces divergent business logic. Fallback: TS
  core on Workers with Rust only for self-hosted/collab (reopens ADR-0006 and the owner direction).

## Deliverables
Results table in ADR-0006 *Confirmation*; binding matrix in `research/`; updated Q-0006.1/Q-0007.1/Q-0044.1/Q-0070.1.

## Result (round 26)

**Conditional pass** (Stage 1 and, on 2026-10-18, Stage 2 on the real platform: the slice 15/15 against Hyperdrive, KV, R2, Queues, Durable Objects and cron after one adapter change; bundle 1.67 MiB, startup 2 to 3 ms. Conditions: the Workflows JS shim, every SQL statement through the single-pipeline typed flow because Hyperdrive failed tokio-postgres' default flow, and Email not exercised; new questions Q-0006.4 to .6). Results, binding matrix and the Stage 2 record: `research/0004-rust-on-workers-binding-matrix.md`. ADR-0006 *Update* has the criteria table and the reopen triggers; the `Send`-future decision is in ADR-0002.
