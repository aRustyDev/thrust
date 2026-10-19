---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0002: Ports and adapters — every Cloudflare primitive sits behind an interface with a self-hosted twin

## Context and Problem Statement

The target is Cloudflare-native, but the user requires that each building block be an interface that supports both Cloudflare-native and fully self-hosted implementations (Queues vs RabbitMQ, Workflows vs Celery-like workers, Durable Objects vs Redis + Hocuspocus, R2 vs S3/MinIO, KV vs Redis, Workers assets vs Caddy/nginx). Switching must be a configuration or compile-time change with at most a redeploy. Today's Plane [S] is already the self-hosted shape (Postgres, Redis/Valkey, RabbitMQ, MinIO, Hocuspocus + Redis), so the self-hosted adapters largely mirror existing infrastructure.

## Decision Drivers

* No vendor lock-in; the self-hosted deployment (docker-compose, `deployments/`) must remain viable.
* Testability: a single contract-test suite per port run against every adapter (ADR-0121).
* Cloudflare-specific semantics (DO single-writer, Queues at-least-once, Workflows durability) must not leak into core code.
* Avoid lowest-common-denominator ports that waste platform strengths.

## Considered Options

* Option 1: Hexagonal architecture — ports as Rust traits in `thrust-ports`, adapters per platform, composition at the edge
* Option 2: Cloudflare-only code, with a separate self-hosted fork if ever needed
* Option 3: Abstraction via a portability framework (e.g. Nitro/unenv-style runtime shims) without explicit domain ports
* Option 4: Self-hosted-first (Node), with Cloudflare as a deployment target of the same server

## Decision Outcome

Chosen option: **Option 1, hexagonal ports and adapters**, because it is the only option that makes both adapter sets first-class and testable. Source: interview 2026-10-17, round 3 (user addition: "keeps these building blocks as interfaces ... a configuration/compilation change and at most a redeploy").

Rules: core and use-case crates depend on no platform crates (`worker`/`wasm-bindgen` bindings, `tokio-postgres`, an AMQP client, etc.); each port ships a shared contract-test suite; ports expose capability flags where semantics differ (e.g. ordering, exactly-once) rather than hiding them.

### Consequences

* Good, because the self-hosted option stays real, and tests run against both.
* Good, because core logic is portable and unit-testable without workerd.
* Bad, because up-front design cost per port and a risk of leaky abstractions.
* Bad, because the contract-test matrix grows with every port × adapter pair (Q-0100).

### Confirmation

ADR-0121 contract suites pass for every adapter; a dependency-graph check (`cargo deny`/`cargo tree`) fails the build if core depends on platform crates.

## Pros and Cons of the Options

### Option 1: Hexagonal

* Good, because explicit, testable, and aligned with the requirement.
* Bad, because discipline is needed to keep ports narrow.

### Option 2: Cloudflare-only

* Good, because it uses platform features directly and has the least code.
* Bad, because it violates the stated requirement.

### Option 3: Runtime shims only

* Good, because less boilerplate.
* Bad, because shims cover runtime APIs, not domain-level capabilities such as workflows and collab.

### Option 4: Self-hosted-first

* Good, because the familiar Node model.
* Bad, because it underuses Durable Objects, Workflows, and Queues semantics.

## Update — interview round 21 (2026-10-18)

Owner direction (round 21): **backend in Rust from the start; Plane's React frontend adopted with minimal changes; a Dioxus (Rust) clone of the UI follows when backend and React frontend are stable, for multi-target builds** (ADR-0013). Ports are Rust traits; adapter sets are cargo-feature bundles per composition root (ADR-0007).

## Update — review fixes, round 25 (2026-10-18)

Port location and boundary rules restated for Rust crates (`thrust-ports`, dependency-graph check). Substance unchanged.

## Update — spike results, round 26 (2026-10-18)

**Port trait shape, decided by spike 0001 (`research/0004-rust-on-workers-binding-matrix.md`, section 3): every port returns a `Send` future** — plain `#[async_trait]`, traits `Send + Sync`, **not** `#[async_trait(?Send)]`. Cloudflare adapters wrap their JS-backed futures in `worker::send::SendFuture`, which is sound because a Worker isolate is single-threaded (workers-rs relies on the same argument for its own bindings). Evidence (`probes/send-shape`, one port and one axum handler compiled in every shape on both targets, `[tested]`): `?Send` fails with `E0277` on both targets once the router is shared, and a `cfg_attr` switch (`?Send` on wasm32 only) fails on wasm32. A shared axum router needs `Send` handler futures on every target, so `?Send` would force two routers (the divergent-logic failure) or a `LocalSet` executor shim. The cost is one wrapper per JS-touching adapter method; the Thrust crates themselves contain no `unsafe`.

**Supporting ports:** `Background` (`spawn`: `tokio::spawn` natively, `ctx.waitUntil` on the Worker, inline in tests), `Clock` and `IdGen`. So core never calls `tokio::spawn`, `Utc::now()` does not exist in core, and uuid has no RNG feature there. **Dependency evidence:** none of 17 runtime or platform crates is in the dependency graph of `thrust-contracts`, `-ports`, `-core`, the shared router or the memory adapter, on either target; `worker`, `wasm-bindgen`, `js-sys` and `web-sys` are absent from the native server and `deadpool-postgres`, `hyper`, `mio`, `socket2` from the Worker. Caveat: `tokio` with features `bytes, io-util, sync` only (no runtime) is in the Worker graph, pulled in by `worker` and `tokio-postgres`.

The sketches that assumed `?Send` (ADR-0045, 0070, 0071, 0080) were corrected in round 26.

## Open Questions

* Q-0006.1 — is workerd/Node runtime portability a hard requirement?
* Q-0007 — compile-time vs runtime adapter selection.
* Q-0100 — CI matrix cost.

## More Information

Port catalogue: ARCHITECTURE.md §1c. Related: [[ADR-0007]], [[ADR-0060]], [[ADR-0070]], [[ADR-0071]], [[ADR-0072]], [[ADR-0080]], [[ADR-0082]].
