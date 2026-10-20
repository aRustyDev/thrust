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

## Generative first

[Rule 9](rules.md) makes generated inputs the default at every layer, with static cases reserved for regressions,
named spec examples and oracle output. Pick the first rung that can express the behaviour:

| Rung | Technique | Generates | Fits best at layer |
|---|---|---|---|
| 1 | Exhaustive matrix ([parameterized](techniques/example-and-parameterized.md), rstest `#[values]`) | every point of a small finite domain: role × permission, status × transition | 1, 3 (role × tenant × route) |
| 2 | [Model-based](techniques/model-based.md) (`model_`) | sequences of operations, checked against a reference model after every step | 2, 3, 4 (the same model over HTTP on every target) |
| 3 | [Property](techniques/property.md) (`prop_`) | values of a stated shape, checked against an invariant | 1, 2 |
| 4 | [Differential](techniques/differential.md) (`oracle_`) | bulk or mutated inputs, compared with an independent implementation | 1, 3 (v1 against pinned Plane) |
| 5 | [Fuzz](techniques/fuzz.md) (`fuzz_replay_`) | coverage-guided bytes for untrusted parsers | 1 |

An exhaustive matrix comes first because, when the domain is small, it checks every input, not a sample. Model-based
comes before property because most of thrust's risk is in state and ordering (X1, X3-X5). The rungs overlap on
purpose: a property states the invariant, a model states the whole behaviour, and a differential run checks both
against an implementation nobody here wrote.

Static tests are the output of this process, not its input: every shrunk counterexample, fuzz crash and fixed bug
becomes a named [pinned case](techniques/example-and-parameterized.md), and the oracle's recorded output becomes the
hermetic [golden corpus](techniques/golden-fixtures.md) that guards the differential run between its live runs.

**Seed policy.** The default loop runs generators on fresh random seeds, so every run searches new inputs. That
is compatible with [Rule 6](rules.md) because the nondeterminism is in the *search*, not in the code under test
(time and randomness are injected), and because a failure is never retried away ([Rule 2](rules.md)): it is a
finding, its seed is persisted in `*.proptest-regressions` and replayed first on every later run ([Rule 7](rules.md)).
`PROPTEST_CASES` sets the budget through the shared config helper: each test's own default (256 in `thrust-core`)
in the loop, 4096 or more under `deep`, and lower for a mutation run if one is too slow (nothing sets that today).
Generated scenarios against a live target (the contract suite's `crates/thrust-contract-tests/tests/model/mod.rs`)
take a small fixed case budget per target, `THRUST_MODEL_CASES` (default 16) rather than `PROPTEST_CASES`, so a
cell's runtime stays bounded.

## Evidence before trust

A pass counts only once something has shown the test can fail ([Rule 8](rules.md), [Evidence and
controls](evidence-and-controls.md)). The live suites already work this way: paired allow/deny assertions, target
and count guards in the scripts, planted mutants. New tests follow the same standard.

## Decisions this strategy implements

ADR-0120, ADR-0121 and ADR-0123 were accepted in round 29 of the plan, and ADR-0122 is still `status: proposed`
(`.agents/plans/contrail/reviews/2026-10-19-fable-architecture-review.md`, FR-17). This chapter describes the parts
that already exist and links the rest.

| ADR | In one line |
|---|---|
| `.agents/plans/contrail/adrs/test/models/0120-test-model-and-oracle.md` | Layered tests with differential scenarios where a schema cannot express the behaviour; each P0/P1 feature maps to at least one test at the right layer. |
| `.agents/plans/contrail/adrs/test/patterns/0121-port-contract-tests-one-suite-n-adapters.md` | One contract suite runs against every adapter; model-based tests for stateful ports. |
| `.agents/plans/contrail/adrs/test/patterns/0122-v1-oracle-and-parity-tests.md` | Public v1 API: schema diff as the PR gate, a curated scenario corpus, a nightly differential run against pinned Plane. |
| `.agents/plans/contrail/adrs/test/standards/0123-test-standards-and-tooling.md` | Toolchain: nextest, proptest, workerd for the wasm32 leg, testcontainers, Playwright; no line-coverage threshold, mutation testing on the permission seam instead (Q-0123.1, round 29). |

Requirements are traced to their test home in `.agents/plans/contrail/TRACEABILITY.md`; see [The TDD
loop](tdd-loop.md).
