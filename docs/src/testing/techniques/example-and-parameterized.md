# Example and parameterized

Status: **live** (`thrust-core`) · Gate: `scripts/rs cargo nextest run`

## Catches

X15 (P), X2 (P for role tables), X20 (B). Two different techniques share the rstest syntax, and
[Rule 9](../rules.md) treats them differently:

- An **exhaustive matrix** (`#[values]`) runs every point of a small finite domain. It is generative: nothing is
  sampled, and nothing is left for someone to think of. It is the first rung of
  [Generative first](../strategy.md#generative-first).
- A **pinned case** (`#[case::name]`) is one input someone fixed. It is static, and it is allowed only as a
  regression, a named spec example or oracle output.

### Exhaustive matrices

Every combination of a few enums or small ranges, checked against an expected value written out as the spec
(a table, a `match` the test owns), never computed by the code under test. Use it when the domain is small enough
to enumerate, for example role × permission, state × transition or a list of status codes. A matrix is exhaustive
only while its `#[values(..)]` lists name every variant. Write the spec `match` with no `_` arm, and guard each
hand-typed list against the enum (`rust-test-logic`, "Matrices"). `authorize_follows_the_role_table` lists its
variants by hand today. If the product grows
past a few hundred tests, or one axis is unbounded (strings, ids), switch to a [property](property.md) over the
same spec table.

### Pinned cases

A named input with a known output, kept for one of three reasons, which its name or a one-line comment states:

| Reason | Name or comment | Example |
|---|---|---|
| regression: a shrunk counterexample, fuzz crash or fixed bug | `#[case::issue_482_trailing_space]`, or `// regression: prop_… counterexample` | `single_dot` in `validate_blob_name_cases` |
| an example the spec, an ADR or Plane names | `// spec: ADR-0080 size limit` | |
| output recorded from an independent oracle | use a [golden fixture](golden-fixtures.md) instead | |

A boundary you can name ("empty", "max length") belongs in the generator as a weighted value, so every run hits it,
rather than in a table. A table of hand-picked inputs with no generator beside it is the gap Rule 9 asks you to fill
or record.

## Use when / Not when

Use a matrix for any rule over a finite domain. Use pinned cases to pin every shrunk counterexample, fuzz crash and
fixed bug, and for examples the spec names. Not as the only test of a behaviour (add the generator), and not for
rendered markup (that is an [app test](app-and-e2e.md)).

## Name and location

No prefix. In-source `#[cfg(test)] mod tests` or the crate's own `tests/` directory ([Rule 5](../rules.md)).
Both forms use `rstest` (a dev-dependency of `thrust-core`); snapshots of plain values use `insta` (not
used yet: the first crate to need it adds it), and a snapshot is a pinned case under the same three reasons.

## Run

```sh
scripts/rs cargo nextest run -p <crate>    # [derived]
```

## Required controls

[Broken implementation](../evidence-and-controls.md#broken-implementation), then [mutation](mutation.md) on the file.
For a matrix, the spec table must be independent of the code: a table read from the implementation passes on any
implementation.

## Owning skill

`.agents/skills/rust-test-logic/SKILL.md` (`.agents/skills/rust-test-logic/references/parameterized-and-snapshots.md`).

## In-repo examples

- `crates/thrust-core/tests/authorize.rs`: `authorize_follows_the_role_table`, an rstest `#[values]` matrix of role by permission
  against a table written out as the spec, not read from the code. This is the shape to copy.
- `crates/thrust-core/src/lib.rs` (`mod tests`): `validate_name_cases` and `validate_blob_name_cases`, rstest tables beside the
  `prop_` validators; `single_dot` is pinned from a property counterexample. Their other cases predate Rule 9 and
  carry no reason label (drift D-15 in [Status](../status.md)).

## Target

Matrices and pinned cases for `thrust-contracts` and `thrust-api` (G-3). The caller × project × route matrix over
HTTP is built: `every_actor_project_and_route_gets_the_status_of_the_access_table` in
`crates/thrust-contract-tests/tests/model/matrix.rs`, attachment routes included.
