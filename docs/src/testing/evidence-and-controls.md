# Evidence and controls

A passing test is evidence only if it could have failed. A **control** is the act that proves it could: break the
thing the test claims to check, watch the test go red, restore it. This page is the catalogue; technique pages and
[by-change](by-change.md) link to its entries. The rule itself is [Rule 8](rules.md).

Controls come in two forms: **one-off**, applied by hand during [step 5 of the loop](tdd-loop.md#5-control) and
reverted; and **durable**, committed so the proof is re-run every time. Prefer durable controls for anything that
guards security, tenancy or uniqueness.

## Paired allow/deny

Every "X is allowed" assertion is paired with a neighbouring "Y is rejected" assertion (cross-tenant, non-member,
guest, duplicate, over-limit, oversize). Where a rejected request could still have changed state, the test also
checks that it had **no effect**. Without the pair, a handler that allows everything, or rejects everything, passes.
Durable. Example: `crates/thrust-contract-tests/tests/contract.rs` (see its header comment and
`cross_tenant_access_is_rejected_and_has_no_effect`).

## Planted mutant

A cargo feature named `mutant_*` that compiles a deliberately broken variant of an adapter; `scripts/mutants.sh`
runs the suite against it and requires failure (exit 101 and at least one `FAILED`). A surviving mutant means some
assertion is vacuous. Durable. Today: `mutant_ignore_tenant`, `mutant_dup_sequence`, `mutant_no_activity_dedupe`
(`crates/thrust-adapters-memory`) and `mutant_discard_noop` (`crates/thrust-adapters-selfhosted`). Add one when a new
invariant guards tenancy, uniqueness, idempotency or transaction safety. See [Mutation](techniques/mutation.md).

## Target guard

Before running the contract suite, `scripts/contract.sh` reads `/_info` and requires the target to report the
expected runtime and adapter set (it exits 5 otherwise). Without it, a run "against Postgres" that actually hit the
memory adapter would pass. Durable. It reads `csrf_enforce` the same way: the main pass needs a server that enforces the
CSRF token and the `csrfoff` pass one that does not, because a CSRF test run against the wrong setting would pass or fail for
the wrong reason (`scripts/mutants.sh` starts a server with the other setting and requires exit 5).

## Count guard

The contract scripts compare the number of passed tests with the expected count for the pass and exit 5 when they
differ, even if `cargo test` exited 0 ("VACUITY GUARD" in `scripts/contract.sh`). Without it, a filter that selects
zero tests reports green. Durable. When you add a contract test, update the matching `EXPECT_*` count at the top
of `scripts/contract.sh` (and in `scripts/stage2-contract.sh` for a pass it runs) in the same commit.

## Non-vacuity

A test asserting that the comparison it relies on actually compared something: enough inputs, non-empty documents,
bytes compared. Durable. Example: `stage3_not_vacuous` in `crates/thrust-editor-doc/tests/stage3.rs`. A
differential or golden test that loads its inputs from a file needs one, and must **fail** when the file is
missing: the `oracle_` tests in `crates/thrust-editor-doc/tests/` do both (resolved drift D-4 in
[Status](status.md)).

## Mutated fixture

Change the expected value, the schema or the comparator and require the test to fail. Durable form:
`mutated_schema_is_detected_by_the_corpus_and_by_the_dump_diff` and
`comparator_discriminates_between_different_documents` in `crates/thrust-editor-doc/tests/controls.rs`.

## Corrupted input

Feed damaged binaries or text and require them to be rejected or detected, never silently decoded as something
else. Durable form: `corrupted_binaries_are_detected` in `crates/thrust-editor-doc/tests/controls.rs`.

## Must-fail case

A test case whose input is known bad and whose assertion is that the code under test catches it. Durable form:
`unsanitised_payload_is_caught` in `crates/thrust-editor-doc/tests/sanitize.rs`.

## Broken implementation

One-off: temporarily replace the implementation with a plausible wrong one (off by one, always `Ok`, wrong
comparison) and require the new test to fail. This is the default control for example and property tests; the
lasting version is cargo-mutants ([Mutation](techniques/mutation.md)).

## Empty input

One-off or durable: remove the seeds, fixtures or corpus a replay test reads and require it to **fail**, not pass
with nothing to check. `rust-test-fuzz` requires a `fuzz_replay_` test to fail when it finds no inputs.

## Generator coverage

The [broken-implementation](#broken-implementation) control proves that a property *can* fail. This one proves
that its generator *reaches* the inputs that matter. A strategy that never produces a cross-tenant actor, a
soft-deleted row or a bare `.` passes on code that is wrong for exactly those inputs. The broken-implementation
control will not notice unless the planted bug happens to fall in a class the generator does reach.

Durable form: an unprefixed test samples the strategy with a fixed-seed `TestRunner::deterministic()`, maps each
value to a small enum of input classes the test owns, and asserts that every class appears at least once. For a
`model_` test the classes depend on state (operation kind, an actor inside or outside the target's tenant, a live
or deleted target), so it samples whole sequences, replays each through the reference model and classifies every
step by the model's state at that point. proptest has no built-in `classify`, so this is the pattern;
the template is in `.agents/skills/rust-test-logic/references/property.md` ("Checking the strategy's reach")
`[verified 2026-10-20: proptest 1.12.0, in a scratch crate]`. Its own control: drop one class from the strategy and
the test must fail (it did). Required for every `model_` test and for any `prop_` test whose strategy is weighted or
filtered. Reach the rare classes by weighting them in the strategy, not by rejecting with `prop_assume!`.

Sample as many inputs as one run of the property draws, not more. A generous sample proves only that the strategy
can reach a class eventually: with 1000 samples the original `thrust-core` validator strategies passed, while at
their own 256 cases they never drew an empty or whitespace-only name or a one-character one, the inputs two of
those properties are about. Where a property discards inputs with `prop_assume!`, assert the share it keeps too.

In-repo: `model_strategy_reaches_every_step_class` in `crates/thrust-contract-tests/tests/model/mod.rs` and in
`crates/thrust-core/tests/model.rs` (over whole sequences), `model_postgres_strategy_reaches_every_step_class` in
`crates/thrust-adapters-selfhosted/tests/model/mod.rs`, `generated_documents_reach_every_kind` in
`crates/thrust-editor-doc/tests/roundtrip.rs`, and `strategies_reach_every_input_class` in
`crates/thrust-core/src/lib.rs`.
