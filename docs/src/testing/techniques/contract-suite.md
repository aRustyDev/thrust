# Contract suite

Status: **live** · Gate: the `contract` jobs in CI (one per cell), or `scripts/contract-all.sh` locally

## Catches

X1, X2, X3, X4, X5, X8, X9, X20 (P); X13 (B, panic containment); X16 (its own guards). This is the backbone of the
repo's evidence: **one** suite of HTTP assertions, run unchanged against every adapter set and both runtimes.

## Use when / Not when

Use for any behaviour a client can observe over HTTP that must be the same on every target: authorization and
tenancy, sequencing, soft delete, outbox delivery, rate limits, attachment authz, error mapping. Not for behaviour
that only one adapter has (`worker_only.rs` holds the Worker-only tests, and adapter internals go in
[real-backend](real-backends.md) tests), and not for pure logic (too slow and too indirect).

## Name and location

`crates/thrust-contract-tests`. The suite only sees `THRUST_BASE_URL` and `THRUST_ADMIN_TOKEN`; it never learns
which target it runs on. Fixtures are created through `POST /internal/seed`, the same route on every target
(`crates/thrust-contract-tests/src/lib.rs`), which is mounted only when `thrust-api` is built with `test_routes`.

- `crates/thrust-contract-tests/tests/contract.rs`: the portable suite.
- `crates/thrust-contract-tests/tests/model/mod.rs`: its generated part, compiled into the same binary and run in the
  `main` pass: random operation sequences, events included, against a reference [model](model-based.md), every
  caller × project × route against the access table, and the generator coverage control. Its
  `sweeper_model_outbox_delivers_what_was_swept_and_nothing_more` runs the same model in the `sweeper` pass, with
  the outbox fast path off.
- `crates/thrust-contract-tests/tests/worker_only.rs`: Durable Object, cron and binding tests for `worker-*` targets.

A pass is selected by **test-name prefix** in `scripts/contract.sh`, not by nextest: `sweeper_`, `panic_`,
`blast_`, `worker_room_`, `worker_cron_`, `worker_bindings_`, `stage2_cron_real_trigger_`. Unprefixed tests in
`contract.rs` form the `main` pass. A new test therefore picks its pass by its name.

## Run

```sh
scripts/contract.sh native-memory           # main pass, 21 tests [verified 2026-10-23: 21 passed]
scripts/contract.sh worker-cf bindings      # [unverified: not run]
scripts/contract-all.sh                     # the 16-cell matrix, serially [unverified: not run]
```

See [Running](../running.md#the-contract-suite) for targets, passes and prerequisites.

## Opt-in cells and the engine pass

Spike 0002 (issue #37) adds two targets and three passes that are **not** in `contract-all.sh`, so they are not required CI checks until ADR-0040 decides the engine: `native-sqlite` (the SQLite adapter on rusqlite) and `worker-do` (the same adapter inside one instance-level Durable Object, under `wrangler dev`). `scripts/contract.sh native-sqlite main` and `worker-do main` run the same 21 tests.
- `engine` (own test binary, `crates/thrust-contract-tests/tests/engine.rs`, tests named `engine_`, so the main pass and `EXPECT_MAIN` are untouched; the target starts with a very high create rate limit): the 50-writer, 10k-create test, search parity with `icontains`, search freshness and scope, and the bulk seed. 4 tests on every target; `scripts/mutants.sh` runs it against `mutant_dup_sequence` on the memory and SQLite adapters, `mutant_no_gate` and `mutant_search_case_sensitive`.
- `do` (`worker-do` only, `worker_only.rs`, tests named `worker_do_`): the `Tx` mapping control on the real `SqlStorage`, the FTS5 trigram probe, the billed-rows probe and the migration story. 5 tests.
- `scale` (a binary, like `bench`): list, search and create latency with 100,000 items seeded through `/internal/seed-work-items`.

`scripts/stage2-contract.sh` runs `main`, `do` and `engine` against a deployed `worker-do` with `THRUST_EXPECT_ADAPTERS=durable`. Numbers and conditions: `research/0011-database-engine-cells.md`.

## Required controls

[Paired allow/deny](../evidence-and-controls.md#paired-allow-deny) in every test,
[target guard](../evidence-and-controls.md#target-guard) and [count guard](../evidence-and-controls.md#count-guard)
in every run, and a [planted mutant](../evidence-and-controls.md#planted-mutant) for each security or uniqueness
invariant. Adding a test means updating the pass's `EXPECT_*` count. The generated tests add
[generator coverage](../evidence-and-controls.md#generator-coverage) and fail when they sent no requests; a
counterexample is saved in `crates/thrust-contract-tests/tests/model/mod.proptest-regressions`, committed
([Rule 7](../rules.md)). `scripts/mutants.sh` sets `PROPTEST_DISABLE_FAILURE_PERSISTENCE`, so the counterexamples
found against its broken builds are not saved.

## Owning skill

**None** (G-1). `rust-test-app` explicitly excludes "HTTP assertions shared across adapters". Until a skill exists,
copy the shape of an existing test in `crates/thrust-contract-tests/tests/contract.rs`: seed with `Fixture::seed`,
assert the allowed case, assert the neighbouring rejected case and its lack of effect, and use `eventually()` for
asynchronous effects. For a new operation or route, extend the generated part first ([Rule 9](../rules.md)): an `Op`
and its rule in the model, or a `Route` in the matrix.

## In-repo examples

`cross_tenant_access_is_rejected_and_has_no_effect`, `concurrent_creates_never_duplicate_a_sequence_id`,
`redelivered_event_is_processed_once`, `worker_bindings_rate_limiting_binding`; generated:
`model_work_items_over_http_match_the_reference_model`,
`every_actor_project_and_route_gets_the_status_of_the_access_table`.

## Target

Every port × adapter green as an acceptance criterion (SPEC §9); capability flags for legitimate adapter
differences (ADR-0121, `.agents/plans/contrail/adrs/test/patterns/0121-port-contract-tests-one-suite-n-adapters.md`).
