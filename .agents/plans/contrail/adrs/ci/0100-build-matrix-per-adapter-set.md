---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0100: CI build and test matrix per adapter set

## Context and Problem Statement

With ports/adapters (ADR-0002) there are at least two adapter sets (Cloudflare, self-hosted), possibly more (lite/SQLite),
and potentially several DB options until ADR-0040 resolves. The core and ports must be proven against every adapter.
Existing CI inherited from Plane: `pull-request-build-lint-web-apps.yml`, `pull-request-build-lint-api.yml` (Django),
`build-branch.yml` (Docker Hub, upstream), `feature-deployment.yml`; fork additions: `woven-build.yml` (GHCR images,
signed+attested), `release-please.yml`, `codeql.yml`, `dependency-review.yml`, `gitleaks.yml`, `scorecard.yml`,
`copyright-check.yml`, `dependabot-auto-merge.yml` **[S]**.

## Decision Drivers

* Every port × every adapter must run the same contract suite (ADR-0121).
* PR latency must stay reasonable; matrix cost is bounded.
* Required status checks are an explicit set: the follow-up must assert required contexts *reported*, not merely "no failures" (a workflow that never starts reads as green).
* Frontend jobs are unchanged for the kept apps (hybrid, ADR-0001).

## Considered Options

* Option 1: Matrix axes `adapter-set × runtime` with path filters; contract suite on all, e2e on a primary pair
* Option 2: Single bundle with runtime DI; run contract tests once per adapter in one job (sequential)
* Option 3: Per-adapter separate workflows (CF workflow, self-hosted workflow), each with own triggers

## Decision Outcome

**Decision (accepted, round 16): Option 1** — a reusable workflow invoked with `adapter-set: [cloudflare, selfhosted]`; PRs run lint/types/unit, Cloudflare-set contract tests and the oracle schema check; nightly runs the full adapter matrix and the differential oracle.

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 1** (confidence: medium): a reusable workflow
`ports-contract.yml` invoked with `adapter-set: [cloudflare, selfhosted]`; CF leg uses the Rust `wasm-bindgen-test` harness on workerd (`@cloudflare/vitest-plugin` applies only to the adopted React/TS packages) and the self-hosted leg uses service containers (Postgres, Valkey, RabbitMQ, MinIO). A single aggregate
job (`ci-required`) is the only required context and asserts that all expected legs *ran* (compare expected vs
reported set).)

### Consequences

* Good, because adapter drift is caught at PR time; adding an adapter set is one matrix entry.
* Bad, because service-container legs are slower; need caching and path filters.
* Neutral: Django `pull-request-build-lint-api.yml` is retired when the Django backend is no longer shipped (only kept as the v1 oracle runner, ADR-0122).

### Confirmation

The aggregate job fails when a matrix leg is skipped/missing; a periodic job diffs ruleset-required contexts against
those reported by a recent run.

## Pros and Cons of the Options

### Option 1: Matrix with path filters

* Good, because explicit, parallel, extensible.
* Bad, because matrix explosion if DB options remain open (DB × adapter-set); mitigate with nightly full-matrix, PR subset.

### Option 2: Single bundle, sequential

* Good, because simplest YAML.
* Bad, because slow, hides which adapter failed, no parallelism.

### Option 3: Separate workflows

* Good, because independent triggers/permissions.
* Bad, because duplicated config; easy for one workflow to silently not run; harder single required-check story.

## Update — interview round 16 (2026-10-17)

Accepted. **PRs run lint/types/unit, Cloudflare-set contract tests and the oracle schema check; nightly runs the full adapter matrix and the differential oracle** (Q-0100, Q-0100.1).

## Update — interview round 21 (2026-10-18)

Matrix gains Rust targets: `wasm32-unknown-unknown` (Worker) and native, plus the pnpm jobs for the adopted React apps.

## Update — review fixes, round 25 (2026-10-18)

Wording fix: the Cloudflare-set test leg is the Rust `wasm32` workerd harness (see ADR-0123); the supply-chain gates gain Rust equivalents: `cargo-deny`, `cargo-vet` or `cargo audit` alongside sha-pinning, scorecard and attestation (ADR-0101).

## Update — review fixes, round 25 (2026-10-18)

Outcome rewritten to the accepted decision; repaired the garbled harness wording introduced by the earlier toolchain fix (Rust `wasm-bindgen-test` on workerd).

## Open Questions

* Q-0100 — matrix shape and cost.
* Q-0100.1 — Nightly full matrix (DB × adapter × runtime) vs PR subset: what is the PR-required subset?
* Q-0100.2 — Retention of Django API CI: keep only for oracle (ADR-0122) or remove?

## More Information

`.github/workflows/*` in this repo; `.github/rulesets/main-branch.json`. Related: [[ADR-0101]], [[ADR-0110]], [[ADR-0121]], [[ADR-0122]].
