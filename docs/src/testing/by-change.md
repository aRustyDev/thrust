# Which tests a change needs

Find the row that matches your change and write **every** test it lists, red first ([The TDD loop](tdd-loop.md)).
A change that matches several rows needs the union. "Control" names an entry in [Evidence and
controls](evidence-and-controls.md); a test without its control is not finished.

Rows marked *(gap)* need a technique whose status is template-only or planned: write the first test of that kind
using the owning skill, or, if no skill owns it, record the missing test under [Status](status.md) and say so in the
PR. Do not silently skip the row.

Two things that are easy to get wrong:

- **Ports are infrastructure traits, not entity stores.** `crates/thrust-ports/src/lib.rs` defines `Database`, `Tx`,
  `Queue`, `EventConsumer`, `BlobStore`, `SessionStore`, `RateLimiter`, `Clock`, `IdGen` and `Background`; work-item
  behaviour reaches storage through `Database` and `Tx`. A new work-item operation is usually a change to
  `thrust-core` plus an HTTP route, and touches a port only if it needs a new `Tx` method.
- **Every public route is under `/api/v1`** (`crates/thrust-api/src/lib.rs`), so any new or changed public route also
  matches the v1 row. `/internal/*`, `/_info` and `/_test/*` exist only when the router is built with `test_routes`
  and are test infrastructure, not API.

The first test of a new kind may need a test-only dev-dependency (`proptest`, `rstest`, `insta`, `datatest-stable`):
adding one to the crate under test is part of the change, so name it in the PR. Installing tools outside Cargo
(a nightly toolchain, `cargo-fuzz`, Playwright) is the user's decision: propose it and wait, as the skills say.

| You are changing... | Write | Controls | Defect classes |
|---|---|---|---|
| **Pure domain logic** in `thrust-core` / `thrust-contracts` (a rule, a calculation, a state transition) | [example cases](techniques/example-and-parameterized.md) for the cases you know; a [property](techniques/property.md) per invariant *(gap: no unit tests yet)* | [broken-implementation](evidence-and-controls.md#broken-implementation) | X15 |
| **A port method** in `thrust-ports` (new or changed signature or semantics) | a [contract-suite](techniques/contract-suite.md) test exercising it over HTTP, paired with its rejection case; a [model-based](techniques/model-based.md) test if the port is stateful (sequence, queue, outbox) *(gap)* | [paired allow/deny](evidence-and-controls.md#paired-allow-deny); [planted mutant](evidence-and-controls.md#planted-mutant) for a security or uniqueness invariant | X1-X5 |
| **A new adapter, or an adapter's behaviour** (`thrust-adapters-*`) | run the whole [contract suite](techniques/contract-suite.md) against the target that uses it; [real-backend](techniques/real-backends.md) tests for transaction, pool or migration behaviour | [target guard](evidence-and-controls.md#target-guard), [count guard](evidence-and-controls.md#count-guard) | X3, X6, X7 |
| **An HTTP route or its authorization** (`thrust-api`) | a [contract-suite](techniques/contract-suite.md) test per allowed role **and** per rejected role/tenant; a router test with `oneshot` for pure request/response mapping *(gap)* | [paired allow/deny](evidence-and-controls.md#paired-allow-deny) | X1, X2, X20 |
| **Any request or response shape under `/api/v1`** (a field, status code, route or error body) | a contract test now; schema diff and differential against pinned Plane per ADR-0122 *(gap: planned)* | [paired allow/deny](evidence-and-controls.md#paired-allow-deny) | X14 |
| **A parser or decoder of outside bytes or text** | a [property](techniques/property.md) `prop_*_never_panics`; a [fuzz](techniques/fuzz.md) harness with a `fuzz_replay_` test and committed seeds | [empty-input](evidence-and-controls.md#empty-input), [broken-implementation](evidence-and-controls.md#broken-implementation) | X13 |
| **Editor-document conversion** (`thrust-editor-doc`: JSON / Yjs / HTML) | [golden-fixture](techniques/golden-fixtures.md) cases in the committed corpus; [differential](techniques/differential.md) runs against the TypeScript oracle | [mutated fixture](evidence-and-controls.md#mutated-fixture), [non-vacuity](evidence-and-controls.md#non-vacuity), [corrupted input](evidence-and-controls.md#corrupted-input) | X11, X13 |
| **The HTML sanitiser** | an XSS-corpus [differential](techniques/differential.md) case; a must-fail case proving unsanitised input is caught | [must-fail case](evidence-and-controls.md#must-fail-case) | X12 |
| **A SQL migration or query** (`thrust-adapters-postgres`) | a [real-backend](techniques/real-backends.md) test in `pg_adapter.rs`; run the `native-pg` and `worker-cf` contract cells | [planted mutant](evidence-and-controls.md#planted-mutant) for transaction behaviour | X6, X7 |
| **A queue, outbox or sweeper path** | contract tests for delivery exactly once and for redelivery; run the `sweeper` and (Worker) `cron` passes | [planted mutant](evidence-and-controls.md#planted-mutant) (`mutant_no_activity_dedupe` is the model) | X5 |
| **A Worker binding or platform behaviour** (`thrust-worker`, `thrust-adapters-cloudflare`) | a `worker_*` test in `worker_only.rs`; run the `worker`, `bindings` and `cron` passes on `worker-cf` | [target guard](evidence-and-controls.md#target-guard) | X8, X9 |
| **A dependency or crate boundary** (any `Cargo.toml`) | run [the architecture gate](techniques/architecture-gates.md) `scripts/check-deps.sh` | it prints its evidence; a forbidden crate fails it | X10 |
| **A Dioxus component** | an SSR [component test](techniques/app-and-e2e.md) selecting by `data-testid` *(gap: no UI crate yet)* | [broken-implementation](evidence-and-controls.md#broken-implementation) | X18 |
| **A port of TypeScript code** | freeze the TypeScript behaviour first with `ts-to-rust`, then [golden](techniques/golden-fixtures.md) or [differential](techniques/differential.md) tests against it | the port must fail its frozen suite before it is written | X11, X14 |
| **A performance-sensitive path** (cold start, request latency) | measure with the [performance](techniques/performance.md) scripts before and after; record numbers in the PR | none yet: measurement only | X19 |

After the row's tests pass, finish with [mutation](techniques/mutation.md) on the crate you changed (step 7 of the
loop).

Defect-class ids (X1-X20) refer to the [defect matrix](defect-matrix.md).
