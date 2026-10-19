---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0006: Backend runtime, language and framework

## Context and Problem Statement

The backend is rewritten from Python/Django (DRF, Celery, channels) to Workers-native services. The language and HTTP framework determine hiring, typing across the client boundary, and the portability of the self-hosted adapter set (ADR-0002). The frontend is TypeScript with shared packages (`@plane/types`, `@plane/services`); `apps/live` is already TypeScript on Node [S].

## Decision Drivers

* Shared types and schemas with the TS frontend (ADR-0030).
* First-class Workers support (bindings, DO, Workflows, Queues).
* The same application code should run on Node/Bun for the self-hosted set (Q-0006.1).
* Team skills and ecosystem for ORM/query, validation, OIDC, SCIM.

## Considered Options

* Option 1: TypeScript on Workers with a lightweight router (e.g. Hono), and a Node/Bun entrypoint for self-hosted (superseded)
* Option 2: Rust compiled to WASM on Workers, plus a native build for self-hosting **(chosen)**
* Option 3: Python Workers (Pyodide) retaining much of the Django-era logic
* Option 4: TypeScript with a heavier framework (NestJS/Fastify) on Node first, Workers as an adapter

## Decision Outcome

**Decision (accepted, round 21): Option 2 — Rust, built as a `wasm32` Cloudflare Worker (workers-rs) and as a native axum/tokio server from the same crates; fallback to a TypeScript core on Workers only if spike 0001 fails.** Non-Rust exceptions only as bounded, documented shims behind a port with an exit plan (round 22). (Earlier hypothesis, superseded: Option 1, TypeScript + Hono.)

### Consequences

* Good, because one language and compile-checked ports across core, adapters and (later) the Dioxus UI (ADR-0013), with small wasm/native builds.
* Bad, because workers-rs is less mature than the JS runtime (Queues beta, D1 alpha, panic=abort, no Tokio; Workflows/Email bindings undocumented for Rust), there is no type sharing with the React frontend, Postgres features (advisory locks, partial indexes) need a Rust data layer that runs on wasm32 (ADR-0044), and CPU-heavy tasks (exports, PDF) need Workflows or a shim.
* Neutral: Python Workers remains experimental and ties code to Cloudflare.

### Confirmation

Spike 0001: implement one vertical slice (work-item create with sequencing + outbox) as shared Rust crates built as a wasm32 Worker (workerd) and as a native server, with identical contract tests passing on both.

## Pros and Cons of the Options

### Option 1: TypeScript + Hono-style router

* Good, because runs on Workers, Node, Bun and Deno; ecosystem fit.
* Bad, because less runtime-enforced type safety than Rust; Node-only libs may not run in workerd.

### Option 2: Rust to WASM

* Good, because performance and strong typing.
* Bad, because no type sharing with the frontend, a smaller ecosystem, and a heavier build chain.

### Option 3: Python Workers

* Good, because logic reuse from Django-era code.
* Bad, because limited library support, not portable, and it carries the Django shape forward.

### Option 4: Node-first framework

* Good, because mature frameworks.
* Bad, because heavier bundles and awkward Workers bindings.

## Update — interview round 9 (2026-10-17)

The owner chose **leave to ADR/spike** for whether Node/workerd portability is a hard requirement (Q-0006.1). At that time no decision was recorded (superseded: round 21 below accepted Rust). Note the hexagonal design (ADR-0002) and compile-time bundles (ADR-0007) already assume a second runtime for self-hosted DB/queue/blob at P0.

## Update — interview round 21 (2026-10-18)

Owner direction (round 21): **backend in Rust from the start; Plane's React frontend adopted with minimal changes; a Dioxus (Rust) clone of the UI follows when backend and React frontend are stable, for multi-target builds** (ADR-0013). **Option 2 (Rust → WASM on Workers, plus a native build for the self-hosted set) is chosen**; the earlier TypeScript/Hono hypothesis is superseded. Portability (Q-0006.1) is now a compile-target property: `wasm32` Worker bundle and native `axum`/`tokio` server from the same crates, selected by cargo features (fits ADR-0007). Known risks to retire in spike 0001: workers-rs maturity (Queues beta, D1 alpha, panic=abort, no Tokio), Workflows/Email bindings not documented for Rust, bundle size (64 MiB) and 1 s startup. Fallback if spike 0001 fails: TypeScript core on Workers (reopens this ADR).

## Update — interview round 22 (2026-10-18)

Non-Rust exceptions (Q-R4): allowed **only as bounded, documented shims behind a port, each with an exit plan** — e.g. a Workflows/Email shim Worker, headless editor conversion (spike 0006), a TypeScript BetterAuth sidecar (spike 0003). Each exception is recorded as a dated note in the owning ADR.

## Update — review fixes, round 25 (2026-10-18)

Decision line, option labels, consequences and Confirmation rewritten to state the accepted Rust outcome; the round-9 'stays proposed' sentence is marked superseded. Substance unchanged.

## Update — spike results, round 26 (2026-10-18)

**Verdict: conditional pass** (spike 0001, `research/0004-rust-on-workers-binding-matrix.md`; code on branch `feat/spike-0001-rust-on-workers` of `aRustyDev/thrust`, six local commits, not pushed). Stage 1 (native server, local workerd and Docker Postgres) passed every criterion it can observe. Stage 2 (a real Cloudflare account plus a reachable Postgres) has **not** run, so the Rust decision stands but is not yet confirmed on the platform. In this note `[tested]` means local workerd or local Postgres, never Cloudflare's network.

| Confirmation criterion (spike 0001) | Result |
|---|---|
| One shared suite green on both targets | **Met.** One HTTP suite (15 main-pass tests, plus sweeper, panic, cron, DO-hibernation and binding passes) ran on four target combinations: native/memory, native/Postgres, Worker/memory, Worker/Cloudflare set. 16 cells, 77 executions, all green |
| Bundle < 20 MiB (platform limit 64 MiB) | **Met locally:** 1.58 MiB for the Cloudflare set |
| Startup < 300 ms (platform limit 1 s) | **Met locally:** 17.7 ms profile window, 10 ms active (local CPU; Cloudflare cold start is a Stage 2 measurement) |
| Every P0 binding native or a thin shim | **Met:** KV, R2, Queues (producer and consumer), Durable Objects (SQLite storage, WebSocket hibernation), Hyperdrive and Cron are native in workers-rs 0.8.7 and were exercised |
| Panic policy defined | **Met** (below) |
| Workflows and Email (conditional) | Workflows needs a JS shim (workers-rs has no Workflow API, ADR-0071); Email is native (local simulation only); AI Gateway is REST |
| Fail condition (a P0 binding unusable, or the dual build forces divergent logic) | **Not triggered:** ports, core, the axum router and the SQL are the same source on both targets |

Non-vacuity: four mutation controls (tenant predicate ignored, duplicate sequence numbers, no event-id dedupe, a pool that recycles a connection still inside a transaction) were each killed by the intended test; one is killed only by a test added because the original "exactly once" test could not have caught a missing dedupe. None of 17 runtime or platform crates appears in `core`, `ports`, `contracts`, the shared router or the memory adapter on either target (`scripts/check-deps.sh`).

**Reopen triggers (any one found in Stage 2 reopens this ADR toward the TypeScript fallback):** (1) `BEGIN … COMMIT` over one Hyperdrive socket is not atomic or fails (sequence allocation and the outbox depend on it); (2) the real Queues consumer cannot ack or retry individual messages from Rust, or text-encoded JSON bodies do not arrive intact; (3) the deployed bundle exceeds 20 MiB or `startup_time_ms` is 300 or more; (4) any use-case needs a `cfg(target_arch = "wasm32")` branch or a second implementation to pass the shared suite.

**Panic policy.** Worker: `panic=abort` plus the workers-rs instance reset. A panic fails its own request with 500; measured locally over five runs, 4–6 % of the other requests suspended on that isolate failed too, and one request per run hung until the client timeout. Therefore: no correctness-bearing state in isolate globals (the in-memory adapters are test-only and not in the Cloudflare bundle); clippy denies `unwrap_used`, `expect_used`, `panic` and `indexing_slicing` in core, ports and adapters (**not yet enforced: a P0 task**); every write is a transaction and every event goes through the outbox, so an abort loses at most an uncommitted transaction; consumers are idempotent on `event_id`. Native: `panic=unwind` plus `CatchPanicLayer`, with no collateral failures measured. `--panic-unwind` on the Worker needs nightly and `-Zbuild-std` and was not built (Q-0006.3).

**Not proven (needs Stage 2, Q-0006.2):** real Hyperdrive behaviour (pooling, upstream TLS, per-request connect cost), Queue retry and DLQ, real cron firing, Cloudflare cold start and network p95, DO eviction timing and billing, KV eventual consistency, real Email delivery, the 128 KB queue ceiling, ORMs on `tokio-postgres`, `thrust-collab-do` as a separate crate, and the Valkey, RabbitMQ and S3 adapters (in-process stand-ins were used on the native side).

**Decision recorded by the spike:** ports return `Send` futures (ADR-0002). **Projected cost:** about $6.4/month on the Cloudflare side at 5 M requests/month (assumptions and arithmetic in the results file, section 9); about $63 at 10x load, plus about $48 for logs unless sampled. The Postgres provider is not priced (Q-G5.1).

## Update — owner decision, round 27 (2026-10-18)

Register of non-Rust exceptions under the round-22 policy (each lives as a dated note in its owning ADR): the Workflows shim (ADR-0071, `entry.mjs`, exit: a Rust Workflow API in workers-rs); the **`space` SSR Worker** (ADR-0011, round 27, feasibility confirmed, about 8.5 pd; it only serves static assets and the per-board `<head>`; exit: the Dioxus public viewer; an SPA plus Rust head injection would remove it, Q-0011.3); headless editor conversion (spike 0006, only if Option C is chosen); a TypeScript BetterAuth sidecar (spike 0003, only if chosen).

## Update — space hosting decision (Q-0011.3, supersedes the SSR Worker), round 27 (2026-10-18)

The **`space` SSR Worker exception is withdrawn** (Q-0011.3, round 27): `space` is built as an SPA and the per-board metadata is injected by Rust (ADR-0011). The register of non-Rust exceptions is therefore the Workflows shim (ADR-0071) plus, only if chosen, headless editor conversion (spike 0006) and a TypeScript BetterAuth sidecar (spike 0003).

## Update — spike 0006 results, round 27 (2026-10-18)

Spike 0006 makes the editor-conversion exception **unlikely**: Option A (Rust, `thrust-editor-doc`) matches the real JS code byte for byte on 1,337 of 1,337 measured pairs, so headless conversion in JavaScript (Option C) stays a documented fallback only if the oracle-CI cost of A is rejected (ADR-0012, Q-0012.3).

## Update — Hyperdrive and the Worker smoke test, round 27 (2026-10-18)

Reopen trigger 1 (a `BEGIN ... COMMIT` over one Hyperdrive socket must be atomic) now has **partial** evidence: a JavaScript Worker (`pg`) ran a transaction and a rollback correctly through Hyperdrive and the Workers VPC service (ADR-0094 round-27 note). The trigger names the Rust path (`tokio-postgres` over `Hyperdrive::connect()`), which is still only proven against local workerd (spike 0001 Stage 1); Stage 2 must repeat it against `thrust-pg-dev`.

## Update — spike 0001 Stage 2 on the real platform, round 28 (2026-10-18)

**Verdict: conditional pass, now with Stage 2 run** (2026-10-18; `research/0004-rust-on-workers-binding-matrix.md` sections 1 and 10; thrust branch `feat/spike-0001-rust-on-workers`, merged as PR aRustyDev/thrust#17). The conditions are the Workflows JS shim, the Hyperdrive statement-flow constraint below, and a real Email send that was not exercised.

* **The slice is green against the real platform:** the shared contract suite 15/15, repeated 8/8, against Hyperdrive (to `thrust-pg-dev`, through a Workers VPC service and a tunnel), KV, R2, Queues, Durable Objects and cron. Also on the real platform: Durable Object WebSocket hibernation with SQLite state (the instance was recycled and the state survived), a Workflow on the real engine through the JS shim, a service binding, a real Secrets Store read, the real cron trigger (an outbox row delivered 53 s after the create), and a failing queue message retried three times and then dead-lettered.
* **Bundle and startup:** 1.67 MiB deployed (568 KiB gzip) against 20 MiB; Worker startup 2 to 3 ms against 300 ms. CPU per invocation averaged 8.35 ms over 6,852 invocations (the cost model assumed about 10.3).
* **Latency, from one US-east laptop:** create p95 296 ms at concurrency 1 and 559 ms at 8; list p95 86 ms; create-to-activity delivery p95 1.84 s; the first database-backed request on a fresh isolate about 0.6 s. These are not from the origin's region and not multi-region.
* **Reopen trigger 1 was hit by the default driver flow and cleared by the single-pipeline flow.** Through Hyperdrive, `tokio-postgres`' default `query`/`execute(&str, …)` (a named prepared statement, then a second round trip) failed on every statement without result columns (0 of 40) and intermittently on the rest (4 to 6 of 8). The adapter now sends every statement as one pipeline on the unnamed statement (`query_typed` / `execute_typed`), which passed 24 of 24 probe cases and the suite. This is a constraint on how Rust talks to Hyperdrive (no server-side statement caching, an explicit type on every parameter, and named-prepared-statement libraries such as sqlx, sea-orm and diesel-async are likely to hit the same failure [unverified]), not a reason to leave Rust. The root cause is not known (no JS-driver differential was run). Whether to record it here and in ADR-0002, and whether to open a Cloudflare support case with the probe, is Q-0006.4.
* **Triggers 2, 3 and 4 were not triggered:** individual `ack` and `retry` with a delay worked and bodies arrived intact; bundle and startup are far inside the limits; no `cfg(target_arch)` and no second implementation (the shared adapter changed for both targets, the local native-pg cell stays green, and all 16 local matrix cells are green).
* **Panic policy:** one request in 328 that was in flight around a panic failed on the real platform (locally 4 to 6 %); `panic=abort` stays, Q-0006.3 stays open.
* **Also learned:** the Rate Limiting binding is approximate (limit 3 allowed 4), so the Durable Object limiter stays for exact limits (ADR-0033, ADR-0082); a fixed window can allow twice the limit across a boundary; a permanent handler error is retried for nothing and an undecodable message is dropped, not dead-lettered (Q-0006.5).
* **Not covered:** Hyperdrive caching on versus off, a 24-hour cost reconciliation, a distant-region bench, a real Email send, KV eventual consistency, Durable Object billing, `--panic-unwind` (Q-0006.6).

## Open Questions

* Q-0006 — which runtime/language. **Answered (round 21): Rust.**
* Q-0006.1 — is workerd/Node portability hard or best-effort. **Answered (round 21): portability is by compile target (wasm32 + native).** Confirmed for core, ports, router and SQL by spike 0001 (round 26).
* Q-0006.2 — Stage 2 of spike 0001: account, Postgres and token scopes (owner).
* Q-0006.3 — Worker panic mode: `abort` plus reset now; `--panic-unwind` (nightly) at P0 planning if the blast radius matters.

## More Information

Related: [[ADR-0002]], [[ADR-0007]], [[ADR-0030]] (superseded by [[ADR-0034]]), [[ADR-0044]].
