---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0120: Test model and the "behaves like Plane" oracle

## Context and Problem Statement

The goal is an equivalent application. We need an explicit test model and a defined *oracle* — what we compare the new
backend against. Existing tests: Django backend pytest suite (run via Docker compose-test), frontend lint/typecheck/unit
in the monorepo **[S]**. The hybrid approach keeps the React apps, so the frontend is a de facto consumer of the new
API; the public `/api/v1` contract is a hard external contract (ADR-0031).

## Decision Drivers

* Parity is the primary correctness criterion for P0/P1 features (ADR-0020).
* Adapter portability is the second (ADR-0002) — needs port contract tests (ADR-0121).
* Tests must run in workerd (the Rust `wasm32` Worker leg) *and* natively / in containers (round 25).
* Cost/time limits on CI (ADR-0100).

## Considered Options

* Option 1: Layered pyramid — unit (core, no I/O; `cargo nextest`, `proptest`) / port-contract (per adapter) / API-contract (v1 oracle) / integration (workerd via `wasm-bindgen-test` or `wrangler dev`, plus testcontainers for native adapters) / e2e (Playwright on the kept UI)
* Option 2: Consumer-driven contracts — the frontend service layer generates the expectations; backend verifies
* Option 3: Differential-only — run identical scenarios against live Plane and the new system and diff

## Decision Outcome

**Decision (accepted, round 29): the leading hypothesis, Option 1 with Option 3 applied selectively: a layered model,
with differential scenarios for the behaviours a schema cannot express; the layers that exist are documented in
`docs/src/testing/strategy.md`. Per FR-17, `reviews/2026-10-19-fable-architecture-review.md` §5.**

Leading hypothesis (accepted): **Option 1 with Option 3 applied selectively** (confidence:
medium): schema/golden contract tests for v1 (ADR-0122); differential scenarios for behaviors that schemas cannot
express (sequencing, activity/notification side effects, soft-delete visibility, filters/grouping).

### Consequences

* Good, because each layer has a clear failure meaning; oracle is concrete.
* Bad, because differential tests need a reachable reference Plane and seeded data.
* Neutral: the core API is new (ADR-0034), so its oracle is behavior (e2e + use-case tests), not wire shape; the `compat-plane` adapter's oracle is golden recordings from the pinned instance (its ≈233 internal routes have no schema).

### Confirmation

Traceability matrix in **`TRACEABILITY.md`** (package root; maintained by the plan owner): each P0/P1 feature in FEATURES.md maps to ≥1 test at the appropriate layer; coverage of that matrix is a release gate. Intentional deviations from Plane are recorded in **`DEVIATIONS.md`**.

## Pros and Cons of the Options

### Option 1: Layered pyramid

* Good, because standard, supports adapters and CI sharding.
* Bad, because says nothing alone about parity — needs an oracle.

### Option 2: Consumer-driven contracts

* Good, because tests what the kept frontend actually uses.
* Bad, because frontend uses only part of v1; internal API redesign makes expectations a moving target.

### Option 3: Differential-only

* Good, because strongest parity evidence.
* Bad, because brittle (ids, timestamps, ordering), expensive, requires live reference.

## Update — interview round 20 (2026-10-18)

Test data: **synthetic seed data** (Q-0120.2). The pinned oracle runs via **docker compose in CI, nightly** (Q-0120.3); Django CI is kept **only as the oracle runner** (Q-0100.2).

## Update — interview round 21 (2026-10-18)

Tooling follows Rust: `cargo test`/nextest, workerd-hosted wasm tests, testcontainers for native adapters, Playwright for e2e; vitest remains only for the adopted React packages.

## Update — review fixes, round 25 (2026-10-18)

Rust toolchain wording and the promised artefacts are made concrete: the traceability matrix lives in `TRACEABILITY.md` and the exception register in `DEVIATIONS.md` (neither exists in SPEC.md, contrary to earlier wording). The oracle covers `/api/v1` only through its schema; the `compat-plane` internal routes and `/auth/*` have no schema and are covered by golden recordings and the nightly differential run (ADR-0122, ADR-0034).

## Update — spike results, round 26 (2026-10-18)

Spike 0008 and bead `plane-b6a`: **the oracle exists.** It is this fork merged with upstream `makeplane/plane` preview `879ce8b5c6` (1.4.2) and pinned by the annotated tag `oracle/v1-pin-20261018` pointing at `ba85d4d3d3`; `oracle/PIN` records the `apps/api` tree sha and `scripts/verify-pin.sh` checks it. The branch `feat/plane-b6a-upstream-sync-oracle` and the tag are **local and not pushed** (Q-0122.4). It runs under docker compose (Postgres, Valkey, RabbitMQ, MinIO, api) from `oracle/compose.oracle.yml` with synthetic seed data only and `ENABLE_DRF_SPECTACULAR=1`; cold build 58 s and cold start 87–90 s make a nightly run cheap. Two findings for the test model: Plane's schema is weaker than its behaviour (12 failure classes, ADR-0122), so recorded behaviour has to carry weight; and the internal `/api/` and `/auth/` have no schema at all, so they are covered by recordings only (the dynamic half of spike 0009, ADR-0034). Q-0120.3 stays answered (compose in CI, nightly); the CI definition is a draft that has not run.

## Update — review round 29 (2026-10-19)

Per FR-17 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now, "every acceptance cites this review as the recorded owner decision"), this ADR is accepted on its leading hypothesis, quoted in the Decision Outcome: "Option 1 with Option 3 applied selectively". Q-0120 is answered. FR-17's evidence is that `docs/src/testing/` has graduated this ADR's content.

* **The layers as built** are in `docs/src/testing/strategy.md` ("Layers"): pure logic, port behaviour, HTTP behaviour, the cross-target matrix and deployed or end-to-end tests, with architecture gates, mutation and controls across all of them. `docs/src/testing/defect-matrix.md` names a primary technique and a backstop for each defect class. The binding rules are `AGENTS.md` §Testing, rules 1 to 8; where the docs and the rules disagree, the rules win.
* **One change from Option 1 as written:** the port-contract layer is one black-box HTTP suite run against four targets, not a suite per port (ADR-0121, round 29).
* **The v1 oracle is not decided here.** Its schema gate, golden corpus and nightly differential run stay in ADR-0122, which stays `proposed`, and Q-0122 stays open with an owner and a date (decided with issue #58's oracle gate). This acceptance settles the layered model only.

## Open Questions

* Q-0120 — model and oracle. (answered round 29, FR-17)
* Q-0120.1 — Which Plane version is the reference (Q-G1), and who hosts a pinned reference instance for differential runs?
* Q-0120.2 — **answered (round 20)**: synthetic seed data.

## More Information

Related: [[ADR-0020]], [[ADR-0031]], [[ADR-0100]], [[ADR-0121]], [[ADR-0122]], [[ADR-0123]].
