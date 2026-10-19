---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0005: Monorepo and package layout

## Context and Problem Statement

The new backend (core, ports, contracts, adapters, composition roots) must live somewhere relative to the retained frontend. This repo is a pnpm + turbo monorepo [S] (`apps/*`, `packages/*`, `catalog:` and `workspace:*` deps, oxlint/oxfmt, release-please, fork CI). The frontend packages (`@plane/editor`, `ui`, `propel`, `i18n`, `types`, `services`, ...) are consumed directly by the apps.

## Decision Drivers

* The hybrid approach keeps `@plane/*` packages; they must stay buildable and consumable.
* Shared types/contracts between server and client (ADR-0034; originally ADR-0030, superseded).
* Fork CI and supply-chain gates already exist here (ADR-0101).
* Upstream-merge friction (Q-G2).

## Considered Options

* Option 1: Extend this repo (superseded) — add new apps/packages beside the Django `apps/api`, retired later
* Option 2: New repo that carries the frontend packages as vendored workspace packages **(chosen; Rust backend as a Cargo workspace)**
* Option 3: New repo for the backend only; this repo remains the frontend and talks to it over the API contract

## Decision Outcome

**Decision (accepted, rounds 15 and 23): Option 2 — a new repo, `aRustyDev/thrust`, that owns the diverging frontend (web/space apps and `@plane/*` copied in as workspace packages, no registry) beside the Rust backend; this fork is frozen as the v1 oracle.** Layout: crates `thrust-*` in a Cargo workspace under `crates/`, pnpm workspace under `web/`. (Earlier hypothesis, superseded: Option 1, extend this repo in place.)

### Consequences

* Good, because the frontend and Rust backend change atomically in one repo with one CI, and this fork stays untouched as the oracle.
* Bad, because two toolchains (Cargo + pnpm) coexist until ADR-0013 Phase 2 retires the React apps; CI and supply-chain gates must be rebuilt for the new repo (ADR-0100, ADR-0101).
* Neutral: Option 3 (backend-only repo) leaves a clean seam but duplicates tooling and slows contract changes.

### Confirmation

Spike 0001: build `thrust-ports` plus a hello-world wasm32 Worker crate and native server crate in the Cargo workspace while the pnpm workspace under `web/` still builds (`pnpm turbo run build` green).

## Pros and Cons of the Options

### Option 1: Extend this repo

* Good, because types are shared directly and CI is reused.
* Bad, because it mixes two backends during the transition.

### Option 2: New repo consuming the frontend as dependencies

* Good, because clean history and licensing boundaries.
* Bad, because the frontend packages are not published as stable libraries, so vendoring is needed.

### Option 3: Backend-only new repo

* Good, because strongest decoupling and independent release cadence.
* Bad, because contract drift and duplicated CI/supply-chain setup.

## Update — interview round 14 (2026-10-17)

Owner chose a **new repo consuming `@plane/*` packages** (Q-0005). Left `proposed` because it sits in tension with ADR-0001/fork-and-diverge for the frontend: Q-0005.2 asks where the forked frontend lives and how `@plane/*` is consumed. This repo becomes the frozen oracle (bead `plane-b6a`) and the frontend source until that is settled.

## Update — interview round 15 (2026-10-17)

Accepted. **The frontend moves into the new repo** (web/space apps and the `@plane/*` packages copied in as workspace packages, no registry); this fork is frozen as the v1 oracle (bead `plane-b6a`) and upstream reference (Q-0005, Q-0005.2). This resolves the tension with fork-and-diverge (ADR-0001): the new repo owns the diverging frontend.

## Update — interview round 21 (2026-10-18)

The new repo exists: **`git@github.com:aRustyDev/thrust.git`** (named for the v1 product, Thrust; MVP codename Contrail). Layout implication of the Rust direction: a **Cargo workspace** (ports, core, adapters-cloudflare, adapters-selfhosted, composition roots) beside a **pnpm workspace** for the adopted React apps and `@plane/*` packages, until ADR-0013 Phase 2 retires them. This planning package should move into the repo (location to confirm, Q-0005.3).

## Update — interview round 23 (2026-10-18)

Layout (Q-R6, Q-0005.3): **single repo `aRustyDev/thrust`; crates prefixed `thrust-*` in a Cargo workspace under `crates/`; pnpm workspace under `web/` for the adopted React apps and `@plane/*` packages; this planning package moves to `docs/plan/` when the repo is first populated.** Codename Contrail survives only in history/docs.

## Update — review fixes, round 25 (2026-10-18)

Decision line rewritten to state the accepted outcome (new repo, Cargo + pnpm layout); the options, consequences and Confirmation no longer describe the superseded in-repo `packages/*` + turbo layout. Substance unchanged.

## Open Questions

* Q-0005 — which layout.
* Q-G2 — is upstream sync a requirement. **Answered (round 4): no.** Upstream-merge friction no longer counts against extending this repo in place; it is a neutral driver.

## More Information

See ARCHITECTURE.md §2a for the proposed layout. Related: [[ADR-0006]], [[ADR-0101]], [[ADR-0110]].
