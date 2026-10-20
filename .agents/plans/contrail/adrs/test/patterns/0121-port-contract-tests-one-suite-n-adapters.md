---
status: accepted
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

**Decision (accepted, round 29): as built in PR #17, one black-box HTTP suite (`thrust-contract-tests`) run against
four targets (`native-memory`, `native-pg`, `worker-memory`, `worker-cf`), with mutation controls (`scripts/mutants.sh`)
and a pass-count guard (`scripts/contract.sh`). Per FR-17, `reviews/2026-10-19-fable-architecture-review.md` §5.**

This keeps Option 1's intent (one authoritative suite that every adapter set must pass) at the HTTP boundary, not as
generic per-port functions in `thrust-ports`. The earlier leading hypothesis, Option 1 + Option 3 for stateful ports,
is narrowed: model-based tests are not part of the contract suite (see the round-29 note).

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

## Update — review round 29 (2026-10-19)

Per FR-17 (`reviews/2026-10-19-fable-architecture-review.md`), this ADR is accepted as built in PR #17 (owner disposition
in review §5), and Q-0121, Q-0121.1 and Q-0121.2 are answered. As built:

* **One HTTP suite.** `crates/thrust-contract-tests` (`tests/contract.rs`, plus `tests/worker_only.rs` for the
  Worker-only passes) depends on `thrust-contracts` only, so it never links the code it tests
  (`docs/src/contributing/crates.md:19`), and it never learns which target it runs on (round 26).
* **Four targets.** `scripts/contract.sh` runs it against `native-memory`, `native-pg`, `worker-memory` and `worker-cf`
  (lines 5 and 92-98), in passes such as `main`, `sweeper`, `panic`, `cron`, `worker` and `bindings`.
  `scripts/contract-all.sh` runs the matrix serially, and CI runs it as 16 cells (`docs/src/architecture/cicd.md:47-50`).
* **Mutation controls.** `scripts/mutants.sh` builds three broken servers (`ignore-tenant`,
  `duplicate-sequence-number`, `no-activity-dedupe`) and one broken Postgres adapter (`discard-noop`); each must make
  the suite exit 101 with at least one failed test (lines 16-44).
* **Count guard.** `scripts/contract.sh` exits 5 when `cargo test` exits 0 but the passed count differs from the
  expected count (lines 245-250), and when the target's `/_info` reports a different target kind or adapter set
  (lines 160-181).

The Confirmation's meta-test is realised by `gate`: it fails unless the set of contract cells that uploaded a pass
marker equals the planned set (`docs/src/architecture/cicd.md:21-28`). Model-based tests (Option 3) live in the crate
they test and follow `docs/src/testing/`, for example `crates/thrust-core/tests/model.rs:144`, added later by PR #31.
Q-0121.1: the legal adapter differences are those of the FR-05 port list: ordering per key, delay ≤ 24 h, message
≤ 128 KB. Q-0121.2: only Stage 2 items need a deployed environment: real cron, DLQ, and hibernation timing.

Per FR-05, the planned matrix shrinks to the ports of ARCHITECTURE §1c (round 29): the Context's `WorkflowEngine`
contract is withdrawn and no RabbitMQ, Valkey or Hocuspocus adapter will join it; the matrix itself is the
`run <target> <pass>` lines of `scripts/contract-all.sh`, which CI's `plan` job reads
(`docs/src/architecture/cicd.md:47-50`), and it never had a cell for any of the four.

## Open Questions

* Q-0121 — suite design. (answered round 29, FR-17)
* Q-0121.1 — Capability-flag vocabulary per port (what differences are legal?). (answered round 29, FR-05, FR-17)
* Q-0121.2 — Minimum fidelity of local queue/workflow emulation in workerd vs real CF (what is only provable in a deployed staging?). (answered round 29, FR-17)

## More Information

CF docs: Workers testing (Vitest integration, test harness), Durable Objects testing guide. Related: [[ADR-0002]], [[ADR-0100]], [[ADR-0120]].
