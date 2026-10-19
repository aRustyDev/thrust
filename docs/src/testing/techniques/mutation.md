# Mutation

Status: **live** (planted mutants, `scripts/mutants.sh`) · cargo-mutants **template-only** · Gate: `scripts/mutants.sh`

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
`mutant_dup_sequence`, `mutant_no_activity_dedupe`) and `crates/thrust-adapters-selfhosted` (`mutant_discard_noop`),
gated with `#[cfg(feature = "mutant_…")]` at the line they break, passed through by `thrust-server`. Each is listed
in `scripts/mutants.sh`. cargo-mutants: configured by `.cargo/mutants.toml` and the `mutants` nextest profile;
`mutants.out*` is never committed.

## Run

```sh
scripts/mutants.sh                                   # needs Docker [unverified: not run]
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

`mutant_ignore_tenant` in `crates/thrust-adapters-memory/src/lib.rs`, killed by
`cross_tenant_access_is_rejected_and_has_no_effect`.

## Target

cargo-mutants `--in-diff` on every PR and a full run on a schedule, once crates have their own tests (G-3); the
planted mutants already run in CI (the `mutants` job). Mutation testing of the permission seam is open question Q-0123.1 in ADR-0123.
