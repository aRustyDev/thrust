# Architecture gates

Status: **live** · Gate: `scripts/check-deps.sh`

## Catches

X10 (P), X8 (B). Checks the dependency graph rather than behaviour: platform and runtime crates must stay out of
the portable core, and the Worker and server graphs must stay separate. A violation compiles and passes every
behavioural test, which is why it needs its own gate.

## Use when / Not when

Run after any change to a `Cargo.toml`, a feature flag or a crate boundary. It does not check code inside a crate
(lints and `cargo deny` are planned for that).

## Name and location

`scripts/check-deps.sh`, the confirmation check for ADR-0002 (ports and adapters) and ADR-0007 (adapter selection).
It uses `cargo tree -i` on both targets and prints its evidence.

## Run

```sh
scripts/check-deps.sh      # [verified 2026-10-19 dd093fa: exit 0]
```

## Required controls

The script prints the full dependency lists it checked; read them. To prove it can fail, temporarily add a
forbidden crate (for example `worker`) to `crates/thrust-core/Cargo.toml` and expect a non-zero exit.

## Owning skill

None.

## In-repo examples

The script itself. `verify-all.sh` runs it as step 3.

## Target

`cargo deny` and a lint for "no platform SDK outside adapters" in CI (`TRACEABILITY.md` R-P1); a build matrix per
adapter set (ADR-0100).
