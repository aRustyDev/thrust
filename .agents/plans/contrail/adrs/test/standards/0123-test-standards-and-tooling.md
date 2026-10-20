---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0123: Test standards and tooling

## Context and Problem Statement

New crates (core, ports, adapters, api, collab) need a consistent test/lint/format standard that works for `wasm32` (workerd) and native builds. The adopted React apps keep the existing monorepo standard (per `AGENTS.md`): pnpm workspaces with `catalog:` deps, turbo, oxlint + oxfmt, strict TypeScript **[S]**. The TypeScript-era Cloudflare tooling (Workers Vitest integration, `createTestHarness()`) now applies only to those React/TS packages.

## Decision Drivers

* Fast inner loop; deterministic CI.
* In-runtime (workerd, `wasm32`) tests for CF adapters; plain native tests for core/self-hosted.
* E2E on the kept React apps against the new backend.
* Match the existing lint/format toolchain for the React apps; Rust uses `clippy`/`rustfmt`; avoid new configuration surface.

## Considered Options

* Option 1: Vitest everywhere (workers plugin for CF adapter/DO tests; Node project for core/self-hosted), Playwright for e2e, oxlint/oxfmt, coverage via v8
* Option 2: Vitest for Workers + Node's built-in test runner for core; Playwright for e2e
* Option 3: Jest + Miniflare directly; Cypress for e2e

## Decision Outcome

**Decision (accepted, round 29): the round-25 revision below, the Rust toolchain, with `docs/src/testing/` and the
`AGENTS.md` testing rules as the graduated standard; mutation testing on the permission seam and no line-coverage
threshold; e2e on the Cloudflare set only. Per FR-17, `reviews/2026-10-19-fable-architecture-review.md` §5.**

**Revised round 25 (Rust backend, ADR-0006):** the toolchain is **`cargo test` / `cargo nextest` for `core` and native adapters, `proptest` for property tests, `wasm-bindgen-test`/workerd (via `wrangler dev` or Miniflare) for the wasm32 Worker leg, testcontainers for self-hosted adapters, Playwright for e2e, `clippy`/`rustfmt` for Rust, oxlint/oxfmt and vitest only for the adopted React packages.** The original TypeScript-era text below (Option 1 'Vitest everywhere', confidence high) is **superseded** and kept for history. Coverage tool (e.g. `cargo-llvm-cov`) and thresholds are planning-time (Q-0123.1; answered round 29: no line-coverage threshold). The spike 0001 harness this waited for exists, and the ADR is accepted (round-29 note).

### Consequences

* Good, because Rust tests share one vocabulary (`cargo nextest`, `proptest`) across core and native adapters; the same suites run in the `wasm32` workerd harness.
* Bad, because the wasm32/workerd harness ties tests to workerd versions (keep `compatibility_date` pinned in test config) and `wasm-bindgen-test` ergonomics are weaker than native tests; two toolchains (cargo + pnpm) coexist until ADR-0013 Phase 2.
* Neutral: coverage gates apply to `core`/`ports`/adapter crates, not generated clients.

### Confirmation

Standards enforced by CI (ADR-0100): `clippy`/`rustfmt`/`cargo check` for both targets plus oxlint/oxfmt/typecheck for the React apps are required; coverage thresholds per crate in config; a repo script verifies every crate has a `test` target and that every adapter crate runs its port's contract suite (ADR-0121).

## Pros and Cons of the Options

### Option 1: Vitest + Playwright (TypeScript-era; superseded for the backend)

* Good, because aligned with CF docs and existing Vite-based frontend.
* Bad, because workers-plugin version churn.

### Option 2: Mixed runners

* Good, because Node runner has no dependency.
* Bad, because two vocabularies; poorer DX.

### Option 3: Jest + Cypress

* Good, because familiar to some.
* Bad, because weaker Workers story; heavier e2e.

## Update — interview round 21 (2026-10-18)

Tooling follows Rust: `cargo test`/nextest, workerd-hosted wasm tests, testcontainers for native adapters, Playwright for e2e; vitest remains only for the adopted React packages.

## Update — review fixes, round 25 (2026-10-18)

Context, Consequences and Confirmation are rewritten for the Rust toolchain; the TypeScript-era options remain only as history and apply to the adopted React packages. The ADR stays `proposed`.

## Update — review round 29 (2026-10-19)

Per FR-17 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now, "every acceptance cites this review as the recorded owner decision"), this ADR is accepted on its leading hypothesis, the round-25 revision in the Decision Outcome: "`cargo test` / `cargo nextest` for `core` and native adapters, `proptest` for property tests, `wasm-bindgen-test`/workerd (via `wrangler dev` or Miniflare) for the wasm32 Worker leg, testcontainers for self-hosted adapters, Playwright for e2e, `clippy`/`rustfmt` for Rust, oxlint/oxfmt and vitest only for the adopted React packages". Q-0123, Q-0123.1 and Q-0123.2 are answered. FR-17's evidence is that `docs/src/testing/` has graduated this ADR's content.

* **The graduated standard is `docs/src/testing/`**, with the binding rules in `AGENTS.md` §Testing (rules 1 to 8, which win where the two disagree) and the profiles in `.config/nextest.toml` and `.cargo/mutants.toml`. This ADR keeps the decision; the docs say what is live today, technique by technique (`docs/src/testing/README.md`, "Where each fact lives"). Live today: nextest, proptest, Postgres through testcontainers, workerd through `wrangler dev`, and mutation testing (`scripts/mutants.sh`, cargo-mutants on `thrust-core`). Not yet: Playwright e2e (planned, `docs/src/testing/techniques/app-and-e2e.md`) and `wasm-bindgen-test` (a target, `docs/src/testing/techniques/real-backends.md`). Clippy and rustfmt are not in CI yet (review §4, "Dependency guard and CI gate"); the clippy deny list and `cargo fmt --check` are issue #40.
* **Coverage (Q-0123.1):** mutation testing on the permission seam, and no line-coverage threshold. This supersedes the Confirmation's "coverage thresholds per crate in config" and the Consequences' Neutral item on coverage gates.
* **E2E (Q-0123.2):** e2e runs on the Cloudflare set only; the self-hosted set's evidence is the contract suite's `native-pg` cell (FR-19).
* **Contract suites:** the Confirmation's "every adapter crate runs its port's contract suite" is superseded by one black-box HTTP suite against four targets, with `gate` failing a pull request unless every planned cell reported (ADR-0121, round 29; `docs/src/architecture/cicd.md`).
* **TypeScript-era text superseded.** Options 1 to 3 (Vitest, Node's runner, Jest and Cypress) and the Context's and Q-0123's citation of `AGENTS.md` for a pnpm, turbo, oxlint and oxfmt standard no longer describe anything: `AGENTS.md` now holds the Rust testing rules and commands. Oxlint, oxfmt and vitest stay only for the adopted React packages, as the round-25 revision says.

## Open Questions

* Q-0123 — standards. (answered round 29, FR-17)
* Q-0123.1 — Coverage thresholds (core vs adapters) and mutation testing for the permission seam? (answered round 29, FR-17)
* Q-0123.2 — Does e2e run against both adapter sets or only the primary? (answered round 29, FR-17)

## More Information

`AGENTS.md` (commands, style). Related: [[ADR-0100]], [[ADR-0120]], [[ADR-0121]].
