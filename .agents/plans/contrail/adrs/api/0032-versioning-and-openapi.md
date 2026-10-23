---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0032: API versioning policy and OpenAPI as source of truth

## Context and Problem Statement

Plane generates an OpenAPI schema from DRF via drf-spectacular (`api/schema/` when enabled) [S]. The new backend must (a) match that schema for `/api/v1` (ADR-0031), and (b) define a versioning policy for the redesigned internal API (ADR-0030). We need to decide whether the schema or the code is authoritative and how versions evolve.

## Decision Drivers

* Contract testing for v1 requires a stable schema artifact.
* Typed clients and docs should be generated, not hand-maintained.
* Internal API must be free to change without external breakage.

## Considered Options

* Option 1: Schema-first — OpenAPI documents are the source; server validators and clients are generated
* Option 2: Code-first — Rust types with derive macros generate OpenAPI (e.g. a `utoipa`-style crate on axum handlers; crate chosen in spike 0008)
* Option 3: Hybrid — v1 contract frozen from Plane (code-first in Rust, constrained by a CI breaking-change diff against the pinned drf-spectacular schema), core API code-first with generated spec

## Decision Outcome

**Decision (accepted, round 24): Option 3 (hybrid)** — `/api/v1` is code-first in Rust, constrained by the frozen Plane drf-spectacular schema (CI breaking-change diff plus the oracle response diff, ADR-0122); the core API is code-first and unversioned, released in lockstep with its clients; the `compat-plane` adapter (ADR-0034) needs no OpenAPI.

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 3** (confidence: medium) because v1's contract is already defined by Plane and must stay frozen, while the internal API benefits from code-first iteration. Versioning: v1 frozen, additive changes only; internal API unversioned but released in lockstep with the frontend (single deployable, ADR-0005).)

### Consequences

* Good, because v1 is protected by a frozen artifact, and the internal API stays agile.
* Bad, because two workflows to maintain.
* Neutral: the lockstep release removes the need for internal versioning but forbids independently deployed old clients.

### Confirmation

CI step diffs the generated v1 OpenAPI against the frozen schema and fails on breaking changes.

## Pros and Cons of the Options

### Option 1: Schema-first

* Good, because the contract is explicit and language-neutral.
* Bad, because slower iteration; generator quality.

### Option 2: Code-first

* Good, because one source of truth in Rust types.
* Bad, because the spec may drift subtly from Plane's v1.

### Option 3: Hybrid

* Good, because matches each surface's needs.
* Bad, because two toolchains.

## Update — interview round 24 (2026-10-18)

Accepted: **Option 3 (hybrid).** `/api/v1` is **code-first in Rust, constrained by the frozen Plane drf-spectacular schema**: CI runs a breaking-change diff against the pinned schema plus the oracle response diff (ADR-0122). The **core API is code-first and unversioned**, released in lockstep with its clients. The `compat-plane` adapter (ADR-0034) needs no OpenAPI; golden recordings protect it. The Dioxus client shares Rust types directly; the generated spec serves scripts, MCP and any TypeScript consumers. The Rust OpenAPI crate and 3.0-vs-3.1 diff tooling are confirmed in spike 0008.

## Update — review fixes, round 25 (2026-10-18)

Options reworded for the Rust backend (no Zod/TypeScript); the accepted hybrid outcome is unchanged.

## Update — spike results, round 26 (2026-10-18)

Spike 0008 inputs for the code-first versus spec-first choice (the hybrid decision stands: v1 code-first with a frozen-schema diff, core API code-first). The pinned schema is OpenAPI 3.0.3 with one `ApiKeyAuthentication` scheme (header `X-API-Key`), 130 operations with unique snake_case verb-first operationIds (the 6 undecorated viewset operations get auto-generated ids and a lowercase `workspaces` tag), 17 tags, 94 component schemas with `…Request` (34), `…Response` (20) and `…Enum` (11) suffixes plus 21 `Paginated*Response` wrappers, and documented statuses 200, 201, 204, 302, 400, 401, 403, 404 and 409. A Rust-generated schema that declares the register's EX-006 fields nullable needs an `oasdiff` exception. **Not done:** the utoipa code-first generation check against this schema (needs the Rust v1 slice; wave 2).

## Update — review round 29 (2026-10-19)

Per FR-06 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now, two
surfaces at P0, the core OpenAPI surface generated at M4), P0 has two HTTP surfaces, compat-plane and `/api/v1`
(ADR-0034, round 29). The core API half of the hybrid Decision is deferred, not changed: the core REST+OpenAPI
surface is generated code-first from the `thrust-core` use-cases at M4, when the first Dioxus client exists, and is
then unversioned and released in lockstep with its clients as decided in round 24. Until M4 there is no generated core
spec; scripts and the P0 AI access use `/api/v1` (F-X08). The `/api/v1` half is unchanged: code-first, constrained by
the frozen drf-spectacular schema and the oracle response diff. The slice's non-Plane routes, today mounted under
`/api/v1/` (`crates/thrust-api/src/lib.rs:269-293`), move out of that prefix (issue #47, M0), so the prefix holds
only wire-compatible routes and the breaking-change diff applies to every route under it. The utoipa generation check
that round 26 left open needs wire-compatible v1 routes; the first are projects, states, labels and work items with
the oracle gate (issue #58, M1).

## Update — issue #47 (2026-10-23)

Issue #47 moved the slice's non-Plane routes out of `/api/v1`, as the round-29 note said it would. They are mounted
under `/api/_slice` (`SLICE_PREFIX` and `slice_path` in `crates/thrust-api/src/lib.rs`; the range
`crates/thrust-api/src/lib.rs:269-293` that the note cites has moved). The `/api/v1` prefix therefore holds no route
today, and the first ones, those of #58, are wire-compatible by construction and gated by the oracle, so the
breaking-change diff will apply to every route under it. ADR-0034 (issue #47 note) has the facts and where the edge
patterns stand.

## Open Questions

* Q-0032 — source of truth and versioning policy.
* Q-0122 — v1 oracle.

## More Information

Related: [[ADR-0030]], [[ADR-0031]], [[ADR-0122]].
