# Architecture gates

Status: **live** · Gates: `scripts/check-deps.sh`, `scripts/check-layout.sh`

## Catches

X10 (P), X8 (B). Checks the dependency graph rather than behaviour: platform and runtime crates must stay out of
the portable core, HTTP surfaces must not point inward from `thrust-compat-plane`, and the Worker and server graphs must stay separate. It also keeps the ORM crates (`sqlx`,
`sea-orm`, `diesel`, `diesel-async`: ADR-0044) out of the whole workspace. A violation compiles and passes every
behavioural test, which is why it needs its own gate.

`scripts/check-layout.sh` checks ADR-0009's layout of `crates/` by directory: a crate in a group that does not name it,
a test fixture or a spike that a shipping crate depends on, `core/` depending on a crate outside it, and a path that a
move left stale where nothing would notice (a workflow's `paths:` filter, `.cargo/mutants.toml`'s globs, a `.gitignore`
line under `crates/`).

## Use when / Not when

Run after any change to a `Cargo.toml`, a feature flag or a crate boundary, and `check-layout.sh` after adding, moving
or removing a crate or a group. It does not check code inside a crate:
clippy's panic lints do (the `lint` job), and `cargo deny` checks licences, advisories and sources and bans the ORM
crates a second time (the `deny` job, configured by `deny.toml`).

## Name and location

`scripts/check-deps.sh`, the confirmation check for ADR-0002 (ports and adapters), ADR-0007 (adapter selection) and
ADR-0044 (no ORM). It uses `cargo tree -i` on both targets and prints its evidence. A compatibility-boundary section
also uses `cargo metadata` to require `thrust-compat-plane` as a workspace member, then asks `cargo tree -e normal
--target all -i thrust-compat-plane` for `thrust-contracts`, `thrust-ports` and `thrust-core`. Only Cargo's explicit
absent replies pass; an error or any other output is `FORBIDDEN`.

The ORM check asks the whole workspace, in any crate, on any kind of edge and in any feature set:
`--workspace --all-features -e all --target all`. The older sections check named crates and only normal edges. It
answers one of three things for each crate: absent, present (the tree starts with that crate), or "cargo could not
say" (offline, a stale lock), which fails with its own message so it is never read as absent.

`scripts/check-layout.sh` reads the tracked files (`git ls-files`) and the workspace (`cargo metadata --no-deps`). A
directory under `crates/` is a crate (a workspace member) or a group; a group holds no file but a `README.md` that
names each directory in it as `` `<name>/` ``, and no crate contains another. No crate has a normal or build dependency
on a crate in `testing/` or `spikes/`, and nothing in `core/` has one on a crate outside `core/`. Dev-dependencies are
allowed in both: a test may use a fixture, and `thrust-core`'s tests run against the memory adapters. Its success
line counts what it read (groups, filter entries, globs, ignore lines), so a check that read nothing shows it.

## Run

```sh
scripts/check-deps.sh      # [verified 2026-10-19 dd093fa: exit 0]
scripts/check-layout.sh               # [verified 2026-11-16: exit 0, 19 crates, 9 groups]
scripts/check-layout.sh --self-test   # 23 cases, each must be accepted or refused as it says
```

## Required controls

The script prints the full dependency lists it checked; read them. To prove it can fail, temporarily add a
forbidden crate (for example `worker`) to `crates/core/usecases/Cargo.toml` and expect a non-zero exit. For the
compatibility boundary, in a disposable checkout add `thrust-compat-plane = { workspace = true }` to
`thrust-core` and expect `FORBIDDEN: thrust-core depends on thrust-compat-plane`; remove the compat crate from
the workspace members and expect the membership guard to exit non-zero.

The ORM check has its own control. Temporarily add `sea-orm` as a dependency of `crates/adapters/postgres`, a
crate the older sections do not name, and run the script: it exits 1, prints `FORBIDDEN: sea-orm is in the workspace
graph (ADR-0044)`, and the tree under it starts at `sea-orm v1.1.20` and names `thrust-adapters-postgres`
`[verified 2026-11-05 7d0074f]`. Two variants show why the flags are what they are. With `sea-orm` as a
`[dev-dependencies]` entry, the script's own query (`-e all`) finds it, and the same query with `-e normal` printed
`nothing to print`, which the script's case arm reads as absent, so `-e normal` alone would have passed it `[verified
2026-11-05 7d0074f]`. With `sea-orm` as an optional dependency behind a feature that nothing enables, the script
fails, because of `--all-features`, and so does `cargo deny check bans` `[verified 2026-11-05 7d0074f]`. Cargo needs
the network for this to resolve; offline, the script reports its `ERROR` line, not a hit.

`check-layout.sh --self-test` is its control, and CI runs it beside the check. It builds a small consistent world, which
must pass, then breaks it one way at a time, and each case must exit 1 with the message of the rule it breaks (not
just any failure). An empty world must exit 2, not 0. On the real tree, a tracked `crates/stray.txt` exits 1
`[verified 2026-11-16]`.

## Owning skill

None.

## In-repo examples

The script itself. `verify-all.sh` runs it as step 3.

## Target

A lint for "no platform SDK outside adapters" in CI (`TRACEABILITY.md` R-P1: clippy's `disallowed-methods` and
`disallowed-types` are the complete check for what `check-deps.sh` does with `grep`, and are not configured yet); a
build matrix per adapter set (ADR-0100).
