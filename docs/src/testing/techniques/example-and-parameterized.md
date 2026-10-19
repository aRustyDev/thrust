# Example and parameterized

Status: **template-only** · Gate: `scripts/rs cargo nextest run`

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
Table-driven cases with `rstest`; snapshots of plain values with `insta`. Neither is a dev-dependency yet: the first
crate to use them adds it.

## Run

```sh
scripts/rs cargo nextest run -p <crate>    # [derived]
```

## Required controls

[Broken implementation](../evidence-and-controls.md#broken-implementation), then [mutation](mutation.md) on the file.

## Owning skill

`.agents/skills/rust-test-logic/SKILL.md` (`.agents/skills/rust-test-logic/references/parameterized-and-snapshots.md`).

## In-repo examples

None yet: no crate has `#[cfg(test)]` modules (measured in [Status](../status.md)). Shapes to copy are in
`.agents/skills/rust-test-logic/references/`.

## Target

Unit tests in `thrust-core` and `thrust-contracts` as their logic grows (G-3).
