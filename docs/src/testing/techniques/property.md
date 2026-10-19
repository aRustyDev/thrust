# Property

Status: **template-only** · Gate: `scripts/rs cargo nextest run` (and the `deep` profile)

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

None yet: `proptest` is not a dependency of any crate. `crates/thrust-editor-doc/tests/decode_panics.rs` hand-rolls
a seeded mutation loop that a `prop_*_never_panics` test would replace.

## Target

Tenant-scope property for R-S1 (`.agents/plans/contrail/TRACEABILITY.md`); proptest is the property tool chosen in
ADR-0123.
