# Property

Status: **live** (`thrust-core`) · Gate: `scripts/rs cargo nextest run` (and the `deep` profile)

## Catches

X15 (P), X13 (P for `never_panics` properties), X1 (planned primary, `TRACEABILITY.md` R-S1), X20 (B). A property
states what must hold for **all** inputs (round trip, idempotence, an invariant, agreement with a simple model) and
lets proptest search for the input that breaks it.

## Use when / Not when

Use when you can state an invariant: encode/decode round trips, "a user never sees rows outside their tenant",
ordering and uniqueness rules, validators. Not for behaviour that only makes sense as a sequence of operations on
state (use [model-based](model-based.md)), and not for untrusted-byte parsers alone (pair with [fuzz](fuzz.md)).

## Name and location

Prefix `prop_` ([Rule 4](../rules.md)): nextest gives it no retries, `flaky-result = "fail"` and a longer timeout.
In the crate's own tests. Shrunk counterexamples land in `*.proptest-regressions` next to the test and are committed
([Rule 7](../rules.md)); each one also becomes a named [example case](example-and-parameterized.md).

## Run

```sh
scripts/rs cargo nextest run -E 'test(/\bprop_/)'                           # [derived]
PROPTEST_CASES=4096 scripts/rs cargo nextest run --profile deep              # [derived]
```

## Required controls

[Broken implementation](../evidence-and-controls.md#broken-implementation): the property must fail, and shrink, on a
plausible wrong implementation. A property that passes on the wrong code is asserting the wrong thing.

## Owning skill

`.agents/skills/rust-test-logic/SKILL.md` (`.agents/skills/rust-test-logic/references/property.md`).

## In-repo examples

`crates/thrust-core/src/lib.rs` (`mod tests`): five `prop_` tests on the validators. `prop_accepted_blob_names_stay_inside_their_scope`
found that `.` was accepted as an attachment name on its first run; the counterexample is committed in
`crates/thrust-core/proptest-regressions/lib.txt` and pinned as the named case `single_dot`. The strategies build valid inputs
by construction (weighted tokens) instead of rejecting them with `prop_assume!`.

`crates/thrust-editor-doc/tests/roundtrip.rs`: documents are generated as a small spec and built with the crate's own
constructors, and every one must pass `Node::check`. JSON and Yjs round trips are identities; the HTML round trip
is not (the parser collapses whitespace, in TypeScript too), so its properties are a fixed point after one round
trip and no text lost but whitespace. Two lessons from mutation testing it: a property that calls the function
under test on both sides (`text_content` here) passes when that function is broken, so the expected value comes
from the spec; and a generator that calls a function to build its input (`Mark::add_to_set`) cannot test it, so
that function gets its own property (`crates/thrust-editor-doc/tests/normalisation.rs`). `decode_panics.rs` stays: it mutates the
recorded corpus, which the generated documents do not resemble.

## Target

The R-S1 tenant-scope property (`.agents/plans/contrail/TRACEABILITY.md`) runs against the memory adapter inside the
`model_` test; the Postgres adapter is the next target. proptest is the property tool chosen in ADR-0123.
