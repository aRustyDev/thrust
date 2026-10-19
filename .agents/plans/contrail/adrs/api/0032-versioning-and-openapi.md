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

## Open Questions

* Q-0032 — source of truth and versioning policy.
* Q-0122 — v1 oracle.

## More Information

Related: [[ADR-0030]], [[ADR-0031]], [[ADR-0122]].
