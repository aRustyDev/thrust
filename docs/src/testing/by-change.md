# Which tests a change needs

Find the row that matches your change and write **every** test it lists, red first ([The TDD loop](tdd-loop.md)).
A change that matches several rows needs the union. "Control" names an entry in [Evidence and
controls](evidence-and-controls.md); a test without its control is not finished.

Rows marked *(gap)* need a technique whose status is template-only or planned: write the first test of that kind
using the owning skill, or, if no skill owns it, record the missing test under [Status](status.md) and say so in the
PR. Do not silently skip the row.

Each row lists the generative test first ([Rule 9](rules.md), [Generative first](strategy.md#generative-first)).
Write that one first. Static cases come after it: pin each counterexample it finds, and add the examples the spec
names, each labelled as such. A row whose generative test is a gap still gets its static tests, plus a recorded gap.

Two things that are easy to get wrong:

- **Ports are infrastructure traits, not entity stores.** `crates/thrust-ports/src/lib.rs` defines `Database`, `Tx`,
  `Queue`, `EventConsumer`, `BlobStore`, `SessionStore`, `RateLimiter`, `Clock`, `IdGen` and `Background`; work-item
  behaviour reaches storage through `Database` and `Tx`. A new work-item operation is usually a change to
  `thrust-core` plus an HTTP route, and touches a port only if it needs a new `Tx` method.
- **Every public route is under `/api/v1` today** (`crates/thrust-api/src/lib.rs`), but these are the slice's own
  routes, not Plane's wire-compatible v1. Finding FR-06 of the architecture review
  (`.agents/plans/contrail/reviews/2026-10-19-fable-architecture-review.md`) reserves the prefix for wire-compatible
  routes, and the slice's routes move out of it ([issue #47](https://github.com/aRustyDev/thrust/issues/47); see
  [Application flow](../flows/app.md)). Until they move, any new or changed route under `/api/v1` also matches the v1
  row. `/internal/*`, `/_info` and `/_test/*` exist only when the router is built with `test_routes` and are test
  infrastructure, not API.

The first test of a new kind may need a test-only dev-dependency (`proptest`, `rstest`, `insta`, `datatest-stable`):
adding one to the crate under test is part of the change, so name it in the PR. Installing tools outside Cargo
(a nightly toolchain, `cargo-fuzz`, Playwright) is the user's decision: propose it and wait, as the skills say.

| You are changing... | Write | Controls | Defect classes |
|---|---|---|---|
| **Pure domain logic** in `thrust-core` / `thrust-contracts` (a rule, a calculation, a state transition) | an exhaustive `#[values]` [matrix](techniques/example-and-parameterized.md#exhaustive-matrices) when the domain is finite (`authorize_follows_the_role_table`), otherwise a [property](techniques/property.md) per invariant; a [model-based](techniques/model-based.md) test for a rule that spans several calls (`crates/thrust-core/tests/` is the pattern); [pinned cases](techniques/example-and-parameterized.md#pinned-cases) for each counterexample and each example the spec names | [broken-implementation](evidence-and-controls.md#broken-implementation) | X15 |
| **A port method** in `thrust-ports` (new or changed signature or semantics) | a [model-based](techniques/model-based.md) test of the port's operations against a reference model if the port is stateful (sequence, queue, outbox) *(gap: G-8 across targets)*; a [contract-suite](techniques/contract-suite.md) test exercising it over HTTP, paired with its rejection case | [paired allow/deny](evidence-and-controls.md#paired-allow-deny); [planted mutant](evidence-and-controls.md#planted-mutant) for a security or uniqueness invariant | X1-X5 |
| **A new adapter, or an adapter's behaviour** (`thrust-adapters-*`) | the [model-based](techniques/model-based.md) test against this adapter set (ADR-0121) *(gap: G-8)*; run the whole [contract suite](techniques/contract-suite.md) against the target that uses it; [real-backend](techniques/real-backends.md) tests for transaction, pool or migration behaviour, as properties where the invariant holds for any input *(gap: G-9)* | [target guard](evidence-and-controls.md#target-guard), [count guard](evidence-and-controls.md#count-guard) | X3, X6, X7 |
| **An HTTP route or its authorization** (`thrust-api`) | a generated role × tenant × route matrix in the [contract suite](techniques/contract-suite.md), every allowed **and** every rejected combination against a table written out as the spec *(gap: G-8; today one hand-written test per case)*; a router test with `oneshot` for pure request/response mapping *(gap)* | [paired allow/deny](evidence-and-controls.md#paired-allow-deny) | X1, X2, X20 |
| **Any request or response shape under `/api/v1`** (a field, status code, route or error body) | a [differential](techniques/differential.md) run and schema diff against pinned Plane per ADR-0122 *(gap: planned)*; until then, a contract test, labelled with the Plane behaviour it pins | [paired allow/deny](evidence-and-controls.md#paired-allow-deny) | X14 |
| **A parser or decoder of outside bytes or text** | a [property](techniques/property.md) `prop_*_never_panics` (and a round trip if there is an encoder); a [fuzz](techniques/fuzz.md) harness with a `fuzz_replay_` test and committed seeds | [empty-input](evidence-and-controls.md#empty-input), [broken-implementation](evidence-and-controls.md#broken-implementation) | X13 |
| **Editor-document conversion** (`thrust-editor-doc`: JSON / Yjs / HTML) | [differential](techniques/differential.md) runs against the TypeScript oracle; a hermetic round-trip [property](techniques/property.md) in Rust *(gap: G-10)*; [golden-fixture](techniques/golden-fixtures.md) cases recorded from the oracle into the committed corpus, plus one for each disagreement found | [mutated fixture](evidence-and-controls.md#mutated-fixture), [non-vacuity](evidence-and-controls.md#non-vacuity), [corrupted input](evidence-and-controls.md#corrupted-input) | X11, X13 |
| **The HTML sanitiser** | a [differential](techniques/differential.md) run over generated and mutated payloads; an XSS-corpus case for each payload it or a report found; a must-fail case proving unsanitised input is caught | [must-fail case](evidence-and-controls.md#must-fail-case) | X12 |
| **A SQL migration or query** (`thrust-adapters-postgres`) | a [real-backend](techniques/real-backends.md) test in `pg_adapter.rs`, as a property or model test where the behaviour holds for any input *(gap: G-9)*; run the `native-pg` and `worker-cf` contract cells | [planted mutant](evidence-and-controls.md#planted-mutant) for transaction behaviour | X6, X7 |
| **A queue, outbox or sweeper path** | a [model-based](techniques/model-based.md) test of publish, redeliver and sweep sequences *(gap: G-8 across targets)*; contract tests for delivery exactly once and for redelivery; run the `sweeper` and (Worker) `cron` passes | [planted mutant](evidence-and-controls.md#planted-mutant) (`mutant_no_activity_dedupe` is the model) | X5 |
| **A Worker binding or platform behaviour** (`thrust-worker`, `thrust-adapters-cloudflare`) | a `worker_*` test in `worker_only.rs`; run the `worker`, `bindings` and `cron` passes on `worker-cf` | [target guard](evidence-and-controls.md#target-guard) | X8, X9 |
| **A dependency or crate boundary** (any `Cargo.toml`) | run [the architecture gate](techniques/architecture-gates.md) `scripts/check-deps.sh` | it prints its evidence; a forbidden crate fails it | X10 |
| **A Dioxus component** | an SSR [component test](techniques/app-and-e2e.md) selecting by `data-testid` *(gap: no UI crate yet)* | [broken-implementation](evidence-and-controls.md#broken-implementation) | X18 |
| **A port of TypeScript code** | freeze the TypeScript behaviour first with `ts-to-rust`, then [differential](techniques/differential.md) tests against it, with [golden](techniques/golden-fixtures.md) fixtures as their hermetic record | the port must fail its frozen suite before it is written | X11, X14 |
| **A performance-sensitive path** (cold start, request latency) | measure with the [performance](techniques/performance.md) scripts before and after; record numbers in the PR | none yet: measurement only | X19 |

After the row's tests pass, finish with [mutation](techniques/mutation.md) on the crate you changed (step 7 of the
loop).

Defect-class ids (X1-X20) refer to the [defect matrix](defect-matrix.md).
