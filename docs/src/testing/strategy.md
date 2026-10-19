# Strategy

thrust is one product built against **ports** (`thrust-ports`) with several **adapter sets** behind them (memory,
Postgres/self-hosted, Cloudflare) and two **runtimes** (native `thrust-server`, wasm32 `thrust-worker` on workerd).
Most of its risk is not in any single function: it is in the same behaviour holding across every adapter set and
both runtimes, in tenancy and authorization never leaking, and in staying compatible with Plane.

The strategy follows from that.

## Layers

Each layer has its own failure meaning. Test at the **lowest layer that can catch the defect**, then let the layers
above it act as backstops.

| Layer | What a failure means | Main techniques |
|---|---|---|
| 1. Pure logic (`thrust-core`, `thrust-contracts`, `thrust-editor-doc`) | a rule or a conversion is wrong | [example](techniques/example-and-parameterized.md), [property](techniques/property.md), [golden](techniques/golden-fixtures.md), [fuzz](techniques/fuzz.md) |
| 2. Port behaviour (one adapter against one port) | an adapter violates its port's contract | [model-based](techniques/model-based.md), [real backends](techniques/real-backends.md) |
| 3. HTTP behaviour (the shared router `thrust-api`) | a route, authz rule or error mapping is wrong | [contract suite](techniques/contract-suite.md), [app tests](techniques/app-and-e2e.md) |
| 4. Cross-target matrix (every adapter set × runtime) | targets diverge | [contract suite](techniques/contract-suite.md) over 4 targets |
| 5. Deployed / end-to-end | the platform or the UI behaves differently from local | stage-2 contract runs, [e2e](techniques/app-and-e2e.md) |
| Across all layers | the architecture or the tests themselves are broken | [architecture gates](techniques/architecture-gates.md), [mutation](techniques/mutation.md), [controls](evidence-and-controls.md) |

## Overlap is deliberate

No single technique is trusted on its own. Every high-risk defect class in the [defect matrix](defect-matrix.md)
names a **primary** technique, the one expected to catch it first and cheaply, and at least one independent
**backstop** that would catch it if the primary test were wrong, missing or vacuous. Examples from the repo today:

- Cross-tenant access is caught by the contract test `cross_tenant_access_is_rejected_and_has_no_effect` (primary);
  the planted mutant `mutant_ignore_tenant` proves that test can fail (backstop on the test itself).
- Editor-document conversion is checked against a committed golden corpus (primary), against a live TypeScript
  differential corpus (backstop), and by controls that mutate the schema and corrupt binaries to prove both
  comparisons can fail.

The overlap is complementary, not redundant: techniques are chosen so that each one misses something different.
Example tests miss inputs nobody thought of (property and fuzz find them); property tests can assert a wrong
invariant (golden and differential tests compare against an independent implementation); every test can pass
vacuously (controls and mutation catch that).

## Evidence before trust

A pass counts only once something has shown the test can fail ([Rule 8](rules.md), [Evidence and
controls](evidence-and-controls.md)). The live suites already work this way: paired allow/deny assertions, target
and count guards in the scripts, planted mutants. New tests follow the same standard.

## Decisions this strategy implements

All four are `status: proposed`; this chapter describes the parts that already exist and links the rest.

| ADR | In one line |
|---|---|
| `.agents/plans/contrail/adrs/test/models/0120-test-model-and-oracle.md` | Layered tests with differential scenarios where a schema cannot express the behaviour; each P0/P1 feature maps to at least one test at the right layer. |
| `.agents/plans/contrail/adrs/test/patterns/0121-port-contract-tests-one-suite-n-adapters.md` | One contract suite runs against every adapter; model-based tests for stateful ports. |
| `.agents/plans/contrail/adrs/test/patterns/0122-v1-oracle-and-parity-tests.md` | Public v1 API: schema diff as the PR gate, a curated scenario corpus, a nightly differential run against pinned Plane. |
| `.agents/plans/contrail/adrs/test/standards/0123-test-standards-and-tooling.md` | Toolchain: nextest, proptest, workerd for the wasm32 leg, testcontainers, Playwright; coverage thresholds still open. |

Requirements are traced to their test home in `.agents/plans/contrail/TRACEABILITY.md`; see [The TDD
loop](tdd-loop.md).
