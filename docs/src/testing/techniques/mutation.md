# Mutation

Status: **live** (planted mutants, `scripts/mutants.sh`; cargo-mutants on `thrust-core`) · Gate: `scripts/mutants.sh` in CI

## Catches

X16 (P): tests that pass without checking what they claim. Mutation testing changes the code and asks whether any
test notices. A surviving mutant is a missing or vacuous assertion.

## Use when / Not when

Use at [step 7 of the loop](../tdd-loop.md#7-mutation) for every change, and when reviewing whether a module's tests
are strong enough. Two mechanisms, for two situations:

| Mechanism | When | How |
|---|---|---|
| **Planted mutants** | the code is only tested through the contract suite or `pg_adapter` (most crates today) | a cargo feature `mutant_<what>` compiles a deliberately broken variant; `scripts/mutants.sh` requires the suite to fail on it |
| **cargo-mutants** | the crate has tests of its own ([Rule 5](../rules.md)) | generates mutants automatically; fix each MISSED one through the public API |

Not a replacement for line coverage reports, and never a reason to write a test aimed at one mutant: strengthen the
invariant the mutant broke.

## Name and location

Planted mutants: features in `crates/thrust-adapters-memory/Cargo.toml` (`mutant_ignore_tenant`,
`mutant_dup_sequence`, `mutant_no_activity_dedupe`, `mutant_ignore_revoked`, which makes a revoked session and a revoked
API key resolve), `crates/thrust-core/Cargo.toml` (`mutant_skip_csrf`, `mutant_unverified_email`, and the two that break the CSRF enforcement switch,
`mutant_csrf_never`, ignored when it is on, killed by the main pass, and `mutant_csrf_always`, ignored when it is off, killed by the
`csrfoff` pass) and
`crates/thrust-adapters-selfhosted` (`mutant_discard_noop`),
gated with `#[cfg(feature = "mutant_…")]` at the line they break, passed through by `thrust-server`. Each is listed
in `scripts/mutants.sh`. cargo-mutants: configured by `.cargo/mutants.toml` and the `mutants` nextest profile;
`mutants.out*` is never committed.

Spike 0002 adds `crates/thrust-adapters-sqlite` features `mutant_dup_sequence`, `mutant_no_gate` (transactions not serialised), `mutant_gate_leak` (an abandoned transaction never reopens the gate) and `mutant_non_atomic_commit` (statements applied one by one), `crates/thrust-adapters-memory` `mutant_search_case_sensitive`, all in `scripts/mutants.sh` (the engine pass, `native-sqlite main` and the crate's own `commit` and `model` tests). `mutant_broken_migration` (also `thrust-worker`) is not a kill control: it builds an object whose migration fails, to show what the platform does with it.

## Run

```sh
scripts/mutants.sh                                   # needs Docker [verified 2026-10-24: 9 KILLED, 2 guard refusals, 0 SocketNotFoundError]
scripts/rs cargo mutants -f <path/to/file.rs>        # [derived]; exit 4 if the crate has no tests of its own
```

## Required controls

The mechanism is itself a control. Check the baseline first: cargo-mutants exit 4 means the unmutated tree failed or
selected nothing, which is not a mutation result.

## Owning skill

`.agents/skills/rust-test-mutation/SKILL.md` for cargo-mutants. Planted mutants have **no owning skill** (G-2); add
one by copying an existing `mutant_*` feature: declare it in the adapter's `Cargo.toml`, pass it through in
`crates/thrust-server/Cargo.toml`, break exactly one invariant under the `cfg`, and add it to the list in
`scripts/mutants.sh`.

## In-repo examples

- Planted: `mutant_ignore_tenant` in `crates/thrust-adapters-memory/src/lib.rs`, killed by the contract test
  `cross_tenant_access_is_rejected_and_has_no_effect`, and by `thrust-core`'s `another_tenants_project_is_not_found`
  and `model_work_items_match_the_reference_model` (enable it with
  `--features thrust-adapters-memory/mutant_ignore_tenant`, a dev-dependency feature).
- cargo-mutants: `cargo mutants -f crates/thrust-core/src/lib.rs` (run with `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1`).
  The first run left 14 MISSED, all in code no test touched yet (outbox relay, attachments, the consumer entrypoint);
  `crates/thrust-core/tests/outbox.rs` and `attachments.rs` test that behaviour through the public API and the
  rerun reports 0 missed. `authenticate`'s only mutant is unviable, so its test has a hand-applied control instead.

## Target

cargo-mutants `--in-diff` on every PR and a full run on a schedule; other crates join once they have tests of their own
(G-3). The planted mutants already run in CI (the `mutants` job). Mutation testing of the permission seam is open question Q-0123.1 in ADR-0123.
