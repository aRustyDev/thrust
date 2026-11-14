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

`crates/thrust-core/tests/assets.rs`: `model_assets_match_the_reference_over_any_sequence`, a sequence of asset steps (the lifecycle: create,
upload, confirm, read, attach, and who may do each) against the memory adapters, compared with a reference model; its
coverage control is `model_strategy_reaches_every_step_class` in the same file. The shared world (fixed clock, counter ids, a
`Background` that panics if used) is `crates/thrust-core/tests/common/mod.rs`.

The core's model of the slice's create, delete and list, `model_work_items_match_the_reference_model`, went with those
use-cases in step 5c-3 of #58, because its subject did. What it wrote down for work items (tenancy, roles, sequence ids that are
never reused, soft delete) is `tests/v1_model/` over HTTP for `/api/v1`, and the core's tests of the v1 use-cases, among them
the generated `prop_work_items_are_numbered_and_placed_one_after_another`. Measured at that step with
`cargo nextest run -p thrust-core --no-fail-fast`, the planted `mutant_ignore_tenant` still fails 40 of the core's tests and
`mutant_dup_sequence` fails 3, so neither depends on the deleted model.

`crates/thrust-contract-tests/tests/model/mod.rs`: `model_work_items_over_http_match_the_reference_model`, the same idea
over HTTP in the contract suite's `main` pass, so it runs unchanged against all four targets. It models what no Plane
route replaces: attachments (ADR-0080: a name is one path segment, an upload over 5 MiB is refused, writers only, the item
must be live to put, and a soft-deleted item's attachments stay readable), the activity feed, and what the queue consumer
does with an event (injected, redelivered, swept). Seven callers (including no token and a bad token), two tenants and a
project that does not exist. Its work items are only the vehicle, made and deleted through `/api/v1` with the actors' API
keys (a delete of a live, an already deleted or an unknown item), because `/api/v1`'s own rules are `tests/v1_model/`'s.
A `Checkpoint` step reads the feed as the project's admin and compares it with the events the model expects, and the
feed is read once more after the run for late rows. `model_strategy_reaches_every_step_class` is its generator coverage
control, classifying every step of 2000 fixed-seed sequences per outbox mode by replaying them through the model and
requiring all 29 classes. It kills the planted `mutant_ignore_tenant` and `mutant_no_activity_dedupe` (`mutant_dup_sequence`
is `v1_model`'s kill now), and a temporary change that let a put land on a soft-deleted item, which no other test in the
`main` pass catches (measured at step 5c-1). A failure shrinks to the few requests that show it. The one counterexample the
old model had saved is the pinned sequence `pinned()` in `mod.rs`, run first by both passes.

The same sequences carry events (ADR-0046). Each committed create and delete is one event, and the model also
injects events straight onto the queue, re-sends injected ones and real outbox events whose ids an earlier
checkpoint learnt, and sweeps. A checkpoint reads both projects' activity: it fails at once on a row no published
event explains (a duplicate, a rejected request's event, an event that should still be in the outbox) and waits up
to 15 s for the missing ones. Every case ends by re-sending every event whose id it knows and checking again, so
each case exercises redelivery whatever the generator drew; a read two seconds after the run catches late duplicates. With the fast path off
(`sweeper_model_outbox_delivers_what_was_swept_and_nothing_more`, the `sweeper` pass) events wait for a sweep, the
sweep must report every waiting row as published, and the outbox depth must equal the number waiting. The
redeliveries kill `mutant_no_activity_dedupe`; running the fast-path-off model against a fast-path-on target fails
on the first sweep.

Attachments are in the same sequences (ADR-0080): puts of empty, small, exactly 5 MiB and one-byte-over bodies,
with and without a content type, under valid, longest and invalid names, on live, deleted, unknown and other
projects' items; gets and deletes of the same. A get must return the stored bytes and content type exactly. The
model keeps each object as its recipe (size, seed, content type) and makes the bytes only to send or compare them.
Each project's caller is usually one who may write there and otherwise anyone, so that sequences build up enough
state for the rare classes (a get after the item's soft delete, a cross-project get) while still covering every
rejection.

`crates/thrust-adapters-selfhosted/tests/model/mod.rs`: `model_postgres_adapter_matches_the_reference_model`, the
same idea one layer down, against the Postgres adapter and a real Postgres. The test chooses every id itself and the
adapter's own values (sequence and sort numbers) are predictable, so the model is a pure planner: each generated
operation becomes a script of port calls, each with the exact answer the adapter must give. Transactions of up to
four steps (create, soft delete, outbox row) commit, roll back, or are dropped unfinished; the model knows that
Postgres gives a rolled-back allocation back, that a conflict can come from a row of the same transaction, and that
an abandoned transaction's writes must not reach the next one. It kills `mutant_discard_noop` (shrunk: a dropped
transaction that wrote an outbox row, then a commit that published it), and a soft delete that ignored the
workspace fails it on a create and a cross-workspace delete. Every call has a 20 s timeout, so a wedged pool fails
instead of hanging.

## Target

Built for work items, their events and attachments (ADR-0121,
`.agents/plans/contrail/adrs/test/patterns/0121-port-contract-tests-one-suite-n-adapters.md`). Still hand-written:
rate limiting and the Worker's `cron` pass (G-8 in [Status](../status.md)).
