# Contract suite

Status: **live** · Gate: `scripts/contract-all.sh` (run by hand; no CI yet)

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
- `crates/thrust-contract-tests/tests/worker_only.rs`: Durable Object, cron and binding tests for `worker-*` targets.

A pass is selected by **test-name prefix** in `scripts/contract.sh`, not by nextest: `sweeper_`, `panic_`,
`blast_`, `worker_room_`, `worker_cron_`, `worker_bindings_`, `stage2_cron_real_trigger_`. Unprefixed tests in
`contract.rs` form the `main` pass. A new test therefore picks its pass by its name.

## Run

```sh
scripts/contract.sh native-memory           # main pass, 15 tests [unverified: not run]
scripts/contract.sh worker-cf bindings      # [unverified: not run]
scripts/contract-all.sh                     # the 16-cell matrix, serially [unverified: not run]
```

See [Running](../running.md#the-contract-suite) for targets, passes and prerequisites.

## Required controls

[Paired allow/deny](../evidence-and-controls.md#paired-allow-deny) in every test,
[target guard](../evidence-and-controls.md#target-guard) and [count guard](../evidence-and-controls.md#count-guard)
in every run, and a [planted mutant](../evidence-and-controls.md#planted-mutant) for each security or uniqueness
invariant. Adding a test means updating the pass's `EXPECT_*` count.

## Owning skill

**None** (G-1). `rust-test-app` explicitly excludes "HTTP assertions shared across adapters". Until a skill exists,
copy the shape of an existing test in `crates/thrust-contract-tests/tests/contract.rs`: seed with `Fixture::seed`,
assert the allowed case, assert the neighbouring rejected case and its lack of effect, and use `eventually()` for
asynchronous effects.

## In-repo examples

`cross_tenant_access_is_rejected_and_has_no_effect`, `concurrent_creates_never_duplicate_a_sequence_id`,
`redelivered_event_is_processed_once`, `worker_bindings_rate_limiting_binding`.

## Target

Every port × adapter green as an acceptance criterion (SPEC §9); capability flags for legitimate adapter
differences (ADR-0121, `.agents/plans/contrail/adrs/test/patterns/0121-port-contract-tests-one-suite-n-adapters.md`).
