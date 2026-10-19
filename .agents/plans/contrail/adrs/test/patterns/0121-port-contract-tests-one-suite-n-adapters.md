---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0121: Port contract tests — one suite, N adapters

## Context and Problem Statement

ADR-0002 promises that swapping adapters is a config/compile change. That promise is only true if every adapter passes
the same behavioral contract: `Queue` (at-least-once, ordering, retry/DLQ), `Collab` (merge, persistence, reconnect),
`BlobStore` (put/get/presign/delete), `Database`/UoW (transactions, sequencing, tenant predicate), `Scheduler`,
`WorkflowEngine` (durability, retries, resume). Cloudflare test tooling (verified in the TypeScript era): the Workers Vitest integration (`@cloudflare/vitest-plugin`) runs tests inside workerd and has Durable Object helpers (`runDurableObjectAlarm`, `evictDurableObject`). **With the Rust backend (ADR-0006) the Cloudflare leg is a `wasm32` workerd harness (`wasm-bindgen-test` or `wrangler dev`/Miniflare driving the compiled Worker); the Vitest plugin applies only to the adopted React/TS packages.** Equivalent DO-eviction checks are done by driving Miniflare's DO storage directly (to confirm in spike 0001).

## Decision Drivers

* One authoritative definition of each port's semantics, living next to the interface.
* Run in workerd (CF adapters, `wasm32` leg) and natively (self-hosted adapters, testcontainers).
* Catch semantic divergences that interfaces cannot express (e.g. ordering, idempotency, transaction isolation).

## Considered Options

* Option 1: Exported conformance suites in the `thrust-ports` crate (a generic `fn port_contract<P: Queue>(factory: impl Fn() -> P)` test module behind a `contract-tests` cargo feature), executed by each adapter crate with its own runner
* Option 2: Per-adapter bespoke tests, reviewed for equivalence by humans
* Option 3: Model-based / property-based conformance (`proptest` state-machine tests vs a reference in-memory model) in addition to Option 1

## Decision Outcome

Decision pending — see Q-0121. Leading hypothesis: **Option 1 + Option 3 for stateful ports** (confidence: medium) —
sequence allocation, queue redelivery, Yjs convergence and outbox atomicity benefit from model-based tests.

### Consequences

* Good, because adding an adapter = passing the suite; regressions in any adapter surface at PR time.
* Bad, because suite must tolerate legitimate adapter differences (declare capability flags, e.g. `strictOrdering: false`).
* Neutral: DO tests use eviction helpers to prove persistence across lifecycle.

### Confirmation

CI matrix (ADR-0100) runs each suite for each registered adapter; a meta-test fails if an adapter registers without
running all suites of its port.

## Pros and Cons of the Options

### Option 1: Exported conformance suites

* Good, because single source of truth; reusable by third-party adapters.
* Bad, because requires careful capability modelling.

### Option 2: Bespoke tests

* Good, because adapter authors tune tests freely.
* Bad, because drift guaranteed; defeats ADR-0002.

### Option 3: Model-based

* Good, because finds interleaving bugs.
* Bad, because harder to write/debug; flaky if time-dependent.

## Update — interview round 21 (2026-10-18)

Tooling follows Rust: `cargo test`/nextest, workerd-hosted wasm tests, testcontainers for native adapters, Playwright for e2e; vitest remains only for the adopted React packages.

## Update — review fixes, round 25 (2026-10-18)

Rust wording: the conformance suites are generic test functions in `thrust-ports` (feature `contract-tests`), run with `cargo nextest` per adapter crate; property/model tests use `proptest`; self-hosted adapters run against testcontainers; the Cloudflare leg runs in a `wasm32` workerd harness. The sibling ADR-0123 records the full toolchain. Port capability flags (e.g. `strict_ordering`) follow the ordering mechanism in ADR-0070 (per-`(workspace, entity)` key). DO-SQLite is not a contract target (desk review only).

## Update — spike results, round 26 (2026-10-18)

Spike 0001 exercised this pattern end to end and found rules worth keeping: the suite is **HTTP-only** and never learns which target it runs on (16 cells over four targets, 77 executions); every "allowed" assertion is paired with a neighbouring must-fail control; the runner **fails if the exit code is 0 but the passed count differs from the expected count**, and asks the target which adapter set it is really running before testing (this guard fired twice during development); and a **mutation-control script** keeps four deliberately broken builds that the suite must fail. Run the matrix serially: parallel `wrangler dev` runs share the build directory and reload each other (measured). Native adapter tests use testcontainers (6 pass). The Worker side runs under wrangler dev with local Hyperdrive, KV, R2 and Queues emulation; real-platform parity is Stage 2.

## Open Questions

* Q-0121 — suite design.
* Q-0121.1 — Capability-flag vocabulary per port (what differences are legal?).
* Q-0121.2 — Minimum fidelity of local queue/workflow emulation in workerd vs real CF (what is only provable in a deployed staging?).

## More Information

CF docs: Workers testing (Vitest integration, test harness), Durable Objects testing guide. Related: [[ADR-0002]], [[ADR-0100]], [[ADR-0120]].
