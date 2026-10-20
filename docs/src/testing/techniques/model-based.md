# Model-based

Status: **live** (`thrust-core`; over HTTP on every contract target) · Gate: `scripts/rs cargo nextest run` (and the
`deep` profile); the `contract` jobs in CI for the HTTP model

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
(`crates/thrust-adapters-memory`) is the natural model for the other adapter sets. A model of behaviour a client sees
over HTTP goes in the contract suite instead (`crates/thrust-contract-tests/tests/model/mod.rs`), so it runs on every
target; there the case budget is `THRUST_MODEL_CASES` (default 16), not `PROPTEST_CASES`.

## Run

```sh
scripts/rs cargo nextest run -E 'test(/\bmodel_/)'     # [derived]
```

## Required controls

[Broken implementation](../evidence-and-controls.md#broken-implementation);
[generator coverage](../evidence-and-controls.md#generator-coverage) over whole sequences; the planted mutants in
`crates/thrust-adapters-memory` (`mutant_dup_sequence`, `mutant_no_activity_dedupe`) should also be killed by any
model test over those ports.

## Owning skill

`.agents/skills/rust-test-logic/SKILL.md` (`.agents/skills/rust-test-logic/references/model-based.md`).

## In-repo examples

`crates/thrust-core/tests/model.rs`: `model_work_items_match_the_reference_model`, a lockstep loop of create, delete and list by
five actors in two tenants against the memory adapters, compared after every step with a reference model that
writes the spec down (validation first, Not Found across tenants, role per operation, sequence ids never reused,
soft delete, every listed row in scope). It kills the planted `mutant_ignore_tenant` and `mutant_dup_sequence`.
The shared world (fixed clock, counter ids, a `Background` that panics if used) is `crates/thrust-core/tests/common/mod.rs`.

`crates/thrust-contract-tests/tests/model/mod.rs`: `model_work_items_over_http_match_the_reference_model`, the same idea
over HTTP in the contract suite's `main` pass, so it runs unchanged against all four targets. Seven callers
(including no token and a bad token), two tenants and a project that does not exist; create with valid, padded,
longest, blank and too-long names and colliding external keys; delete of live, deleted, unknown and other-project
items; list in every scope. The model writes down the order of the checks (`401`, then `400`, then tenancy and role),
that a failed create may leave at most one sequence gap (ADR-0043: memory leaves one, Postgres does not), and that a
soft delete frees an external key. `model_strategy_reaches_every_step_class` is its generator coverage control,
classifying every step of 500 fixed-seed sequences by replaying them through the model. It kills the planted
`mutant_ignore_tenant` and `mutant_dup_sequence`, and a failure shrinks to the few requests that show it.

## Target

Built for work items (ADR-0121, `.agents/plans/contrail/adrs/test/patterns/0121-port-contract-tests-one-suite-n-adapters.md`).
Still hand-written: outbox, queue and sweeper sequences, and attachments (G-8 in [Status](../status.md)).
