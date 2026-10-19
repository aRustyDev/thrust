# Example and parameterized

Status: **live** (`thrust-core`) · Gate: `scripts/rs cargo nextest run`

## Catches

X15 (P), X20 (B). The cheapest test there is: a named input with a known expected output. A table of `#[case]`s
documents the edge cases you already know about; every bug found by any other technique comes back here as a named
case.

## Use when / Not when

Use for pure logic with known edge cases, for each error variant a function can return, and to pin every shrunk
counterexample from a property test or fuzz crash. Not as the only test of an invariant that holds for all inputs
(add a [property](property.md)), and not for rendered markup (that is an [app test](app-and-e2e.md)).

## Name and location

No prefix. In-source `#[cfg(test)] mod tests` or the crate's own `tests/` directory ([Rule 5](../rules.md)).
Table-driven cases with `rstest` (a dev-dependency of `thrust-core`); snapshots of plain values with `insta` (not
used yet: the first crate to need it adds it).

## Run

```sh
scripts/rs cargo nextest run -p <crate>    # [derived]
```

## Required controls

[Broken implementation](../evidence-and-controls.md#broken-implementation), then [mutation](mutation.md) on the file.

## Owning skill

`.agents/skills/rust-test-logic/SKILL.md` (`.agents/skills/rust-test-logic/references/parameterized-and-snapshots.md`).

## In-repo examples

- `crates/thrust-core/src/lib.rs` (`mod tests`): `validate_name_cases` and `validate_blob_name_cases`, rstest tables with one named
  case per boundary and `single_dot` pinned from a property counterexample.
- `crates/thrust-core/tests/authorize.rs`: `authorize_follows_the_role_table`, an rstest `#[values]` matrix of role by permission
  against a table written out as the spec, not read from the code.

## Target

The same for `thrust-contracts` and `thrust-api` (G-3).
