# Model-based

Status: **template-only** · Gate: `scripts/rs cargo nextest run` (and the `deep` profile)

## Catches

X3, X4, X5 (planned primary for stateful ports), X15 (B). Generates random **sequences** of operations, runs them
against the real implementation and a trivially correct in-memory model in lockstep, and fails on the first
disagreement.

## Use when / Not when

Use for anything stateful where the bug is in the order of operations: sequence allocation, soft delete, outbox and
redelivery, CRUD over a port. Not for a single request/response (an [app test](app-and-e2e.md) or a
[contract test](contract-suite.md)), and not for concurrency interleavings (out of scope of the skill).

## Name and location

Prefix `model_`: same nextest policy as `prop_`. In the crate that owns the port implementation. The memory adapter
(`crates/thrust-adapters-memory`) is the natural model for the other adapter sets.

## Run

```sh
scripts/rs cargo nextest run -E 'test(/\bmodel_/)'     # [derived]
```

## Required controls

[Broken implementation](../evidence-and-controls.md#broken-implementation); the planted mutants in
`crates/thrust-adapters-memory` (`mutant_dup_sequence`, `mutant_no_activity_dedupe`) should also be killed by any
model test over those ports.

## Owning skill

`.agents/skills/rust-test-logic/SKILL.md` (`.agents/skills/rust-test-logic/references/model-based.md`).

## In-repo examples

None yet. Templates: `kv_store_model.rs` and `api_model.rs` under `.agents/skills/rust-test-logic/references/examples/`.

## Target

Model-based tests for stateful ports, the leading option in ADR-0121
(`.agents/plans/contrail/adrs/test/patterns/0121-port-contract-tests-one-suite-n-adapters.md`).
