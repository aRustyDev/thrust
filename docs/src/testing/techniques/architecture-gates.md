# Architecture gates

Status: **live** · Gate: `scripts/check-deps.sh`

## Catches

X10 (P), X8 (B). Checks the dependency graph rather than behaviour: platform and runtime crates must stay out of
the portable core, and the Worker and server graphs must stay separate. It also keeps the ORM crates (`sqlx`,
`sea-orm`, `diesel`, `diesel-async`: ADR-0044) out of the whole workspace. A violation compiles and passes every
behavioural test, which is why it needs its own gate.

## Use when / Not when

Run after any change to a `Cargo.toml`, a feature flag or a crate boundary. It does not check code inside a crate:
clippy's panic lints do (the `lint` job), and `cargo deny` checks licences, advisories and sources and bans the ORM
crates a second time (the `deny` job, configured by `deny.toml`).

## Name and location

`scripts/check-deps.sh`, the confirmation check for ADR-0002 (ports and adapters), ADR-0007 (adapter selection) and
ADR-0044 (no ORM). It uses `cargo tree -i` on both targets and prints its evidence.

The ORM check asks the whole workspace, in any crate, on any kind of edge and in any feature set:
`--workspace --all-features -e all --target all`. The older sections check named crates and only normal edges. It
answers one of three things for each crate: absent, present (the tree starts with that crate), or "cargo could not
say" (offline, a stale lock), which fails with its own message so it is never read as absent.

## Run

```sh
scripts/check-deps.sh      # [verified 2026-10-19 dd093fa: exit 0]
```

## Required controls

The script prints the full dependency lists it checked; read them. To prove it can fail, temporarily add a
forbidden crate (for example `worker`) to `crates/thrust-core/Cargo.toml` and expect a non-zero exit.

The ORM check has its own control. Temporarily add `sea-orm` as a dependency of `crates/thrust-adapters-postgres`, a
crate the older sections do not name, and run the script: it exits 1, prints `FORBIDDEN: sea-orm is in the workspace
graph (ADR-0044)`, and the tree under it starts at `sea-orm v1.1.20` and names `thrust-adapters-postgres`
`[verified 2026-11-05 7d0074f]`. Two variants show why the flags are what they are. With `sea-orm` as a
`[dev-dependencies]` entry, the script's own query (`-e all`) finds it, and the same query with `-e normal` printed
`nothing to print`, which the script's case arm reads as absent, so `-e normal` alone would have passed it `[verified
2026-11-05 7d0074f]`. With `sea-orm` as an optional dependency behind a feature that nothing enables, the script
fails, because of `--all-features`, and so does `cargo deny check bans` `[verified 2026-11-05 7d0074f]`. Cargo needs
the network for this to resolve; offline, the script reports its `ERROR` line, not a hit.

## Owning skill

None.

## In-repo examples

The script itself. `verify-all.sh` runs it as step 3.

## Target

A lint for "no platform SDK outside adapters" in CI (`TRACEABILITY.md` R-P1: clippy's `disallowed-methods` and
`disallowed-types` are the complete check for what `check-deps.sh` does with `grep`, and are not configured yet); a
build matrix per adapter set (ADR-0100).
