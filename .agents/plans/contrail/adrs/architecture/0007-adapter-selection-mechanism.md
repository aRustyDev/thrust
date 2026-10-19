---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0007: Adapter selection mechanism (compile-time vs runtime configuration)

## Context and Problem Statement

ADR-0002 requires that swapping a Cloudflare primitive for its self-hosted twin is a configuration or compile-time change plus at most a redeploy. We must decide how adapters are bound to ports: separate bundles per adapter set, or a single bundle that chooses by configuration. Cloudflare-only constructs (DO classes, Workflow classes, bindings declared in wrangler config) cannot exist in a native build, and native-only crates cannot be compiled into a Worker.

## Decision Drivers

* Bundle size and workerd compatibility (no Node-only imports in the CF bundle).
* Mixed deployments within a platform bundle, e.g. CF Workers plus an external Postgres, or the native server with R2 as the blob store.
* Fail-fast on invalid combinations.
* Simplicity of CI and release (ADR-0100).

## Considered Options

* Option 1: Compile-time — one composition-root crate per adapter set (`thrust-worker` for wasm32/Cloudflare, `thrust-server` for native), adapters selected by cargo features **(chosen)**
* Option 2: Runtime — single bundle, typed `AppConfig` selects adapters via dependency injection by environment
* Option 3: Hybrid — per-platform composition roots, each with runtime config selecting among adapters valid on that platform

## Decision Outcome

Chosen option: **Option 1 — compile-time bundles per adapter set**, because the owner selected it in interview round 6 (2026-10-17) over the hybrid this ADR had led with. Each composition-root crate (`thrust-worker` for Cloudflare, `thrust-server` for self-hosted) wires a fixed adapter set via cargo features; an invalid combination fails at build, and bundles carry no unused Cloudflare or Node code.

Accepted trade-off: swapping a single adapter *within* a platform (e.g. Hyperdrive→D1, SMTP→CF Email) is a rebuild and redeploy, not a runtime flag. That still satisfies ADR-0002 ("config/compile-time change plus at most a redeploy"). The superseded hybrid hypothesis (build the set, override per port at runtime) is rejected for now; reopen only if the DB spike (ADR-0040) shows the DB engine must be switchable per environment without a rebuild.

### Consequences

* Good, because the CF bundle never contains native-only code. Mixing adapters *within* one platform bundle (e.g. Worker + external Postgres) stays possible because both are compiled into that bundle; arbitrary runtime mixing is not.
* Bad, because two composition roots to maintain and a validity matrix to test.
* Neutral: a typed `AppConfig` with startup validation rejects unsupported combos.

### Confirmation

CI builds both composition-root crates; a dependency-graph check (`cargo tree`/`cargo deny`) asserts the wasm32 build has no native-only crates; a startup test asserts invalid combos fail fast.

## Pros and Cons of the Options

### Option 1: Compile-time

* Good, because the smallest bundles and static guarantees.
* Bad, because every adapter permutation needs its own build, and mixed setups multiply builds.

### Option 2: Runtime

* Good, because one artifact and the most flexible deployments.
* Bad, because the bundle includes unused adapters, and CF-only classes cannot be declared in a Node build.

### Option 3: Hybrid

* Good, because balances size, safety, and flexibility.
* Bad, because more moving parts.

## Update — review fixes, round 25 (2026-10-18)

Option labels corrected (compile-time bundles = Option 1, not Option 3) and composition roots restated as Rust crates with cargo features. Substance unchanged.

## Update — spike results, round 26 (2026-10-18)

**Q-0007.1 answered (spike 0001, `[tested]` by `scripts/check-deps.sh`):** Cloudflare-only bindings are simply not compiled into the self-hosted bundle. The native server's dependency graph contains no `worker*`, `wasm-bindgen`, `js-sys` or `web-sys` crate, and the Worker bundle (built with `--no-default-features`) contains no in-memory adapter. Adapter sets are chosen with cargo features at the composition roots, as designed. Constraint found: **Durable Object classes must be exported from the same Wasm module as `fetch`**, so a DO class cannot live in a separately built crate of the same Worker (see ADR-0090).

## Open Questions

* Q-0007 — selection mechanism. **Answered (round 6): compile-time bundles.**
* Q-0007.1 — how CF-only bindings (DO, Workflows) are declared when building the self-hosted bundle.
* Q-0100 — CI matrix cost.

## More Information

Related: [[ADR-0002]], [[ADR-0006]], [[ADR-0100]], [[ADR-0092]].
