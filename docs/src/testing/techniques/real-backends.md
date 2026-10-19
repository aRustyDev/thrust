# Real backends

Status: **live** (Postgres via testcontainers; workerd via `wrangler dev`) · Gate: `scripts/verify-all.sh`

## Catches

X6, X7 (P), X3, X4, X5 (B). Tests an adapter against the real thing it adapts (a Postgres server, the workerd
runtime) instead of a fake, to catch behaviour that only exists there: transactions, constraint violations, pool
reuse, migrations, binding semantics.

## Use when / Not when

Use for adapter internals that HTTP cannot observe directly: an abandoned transaction not poisoning the pool,
rollback atomicity, a unique violation mapped to a conflict, the outbox committed with its entity. Not for behaviour
the [contract suite](contract-suite.md) can already see over HTTP, and never in the default nextest profile
([Rule 6](../rules.md)).

## Name and location

`crates/thrust-adapters-selfhosted/tests/pg_adapter.rs` (`#[tokio::test]`, starts a Postgres container through
`testcontainers`). It is excluded from the default profile by `binary(pg_adapter)`. The Worker side is exercised
through the contract suite on the `worker-*` targets, which run on local workerd via `scripts/worker-dev.sh`.

## Run

```sh
scripts/rs cargo nextest run --ignore-default-filter -E 'binary(pg_adapter)'   # needs Docker [unverified: not run]
```

`scripts/verify-all.sh` runs it with `cargo test` as step 4.

## Required controls

[Planted mutant](../evidence-and-controls.md#planted-mutant): `mutant_discard_noop` must make
`an_abandoned_transaction_does_not_poison_the_pool` fail (`scripts/mutants.sh` checks it). The rollback test carries
its own control.

## Owning skill

**None** (G-1). A parked draft (`.agents/plans/project-skills/artifacts/skills/rust-testing`) covers testcontainers
but is not installed.

## In-repo examples

`membership_distinguishes_member_non_member_and_other_tenant`, `concurrent_allocation_is_unique_contiguous_and_serialised`,
`rollback_is_atomic_and_unique_violation_is_a_conflict`, `outbox_is_atomic_with_the_entity_and_dispatch_is_recorded`.

## Target

testcontainers for self-hosted adapters and `wasm-bindgen-test`/workerd for the wasm32 leg: ADR-0123
(`.agents/plans/contrail/adrs/test/standards/0123-test-standards-and-tooling.md`).
