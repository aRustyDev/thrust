# Example cases and snapshots (rstest, insta)

Tags: `[re-run]` reproduced on this machine; `[zip]` recorded by the source's author, not re-run; `[derived]` read
from crate docs or source, not run; `[unverified: <why>]` not run.

Use example cases for inputs a person chose on purpose: boundaries, known tricky values, examples from the spec or
the old TypeScript tests, and **one named case for every bug fixed**. They prove only the cases listed, so pair them
with a `prop_` test (`property.md`) for the rest of the input space.

Add `rstest = "0.26"` to `[dev-dependencies]` `[zip]` (0.26.1 resolved `[re-run]`). Every case becomes its own test,
for example `tests::isqrt_examples::case_3_below_square` `[zip]`, so nextest reports, filters and reruns each case
separately. A `for` loop inside one test stops at the first failure and hides the rest.

## Named cases

```rust
use rstest::rstest;

#[rstest]
#[case::zero(0, 0)]
#[case::one(1, 1)]
#[case::below_square(15, 3)]
#[case::square(16, 4)]
#[case::max(u64::MAX, 4_294_967_295)]
fn isqrt_examples(#[case] n: u64, #[case] expected: u64) {
    assert_eq!(isqrt(n), expected);
}
```

`isqrt` is an example function. Always name cases (`#[case::description(..)]`): the name is what shows up in a
failure. A regression case is `#[case::issue_482_trailing_space(..)]` with a one-line comment linking the issue; never
remove one without removing the behaviour it pins. A shrunk proptest counterexample becomes one of these.

Put expected errors in the table instead of using `#[should_panic]`:

```rust
#[rstest]
#[case::empty(&[], Err(FrameError::TooShort))]
#[case::header_only(&[7, 0, 0], Ok(Frame { tag: 7, payload: vec![] }))]
#[case::short_payload(&[7, 0, 2, 1], Err(FrameError::LengthMismatch { declared: 2, actual: 1 }))]
fn decode_examples(#[case] bytes: &[u8], #[case] expected: Result<Frame, FrameError>) {
    assert_eq!(decode(bytes), expected);
}
```

## Async cases

Put the runtime attribute after `#[rstest]`; each case becomes its own `#[tokio::test]` (dev-dependency `tokio` with
`macros` and `rt`) `[derived]`.

```rust
#[rstest::rstest]
#[case::existing(7, Some("widget"))]
#[case::missing(8, None)]
#[tokio::test]
async fn lookup_item(#[case] id: u32, #[case] expected: Option<&str>) {
    let store = ItemStore::with(&[(7, "widget")]);
    assert_eq!(store.get(id).await.as_deref(), expected);
}
```

`ItemStore`, `with` and the async `get` are example names for your own async unit. Status tables for axum routes are
`rust-test-app`'s job, not this skill's.

## Matrices

Each value of a `#[values]` argument runs as a separate test (`unknown_ids_are_none::id_3_u32__MAX`, the zip's
naming `[zip]`). Several `#[values]` arguments multiply: 5 x 5 values is 25 tests. Keep matrices small; if you want
"every input", write a property test.

```rust
#[rstest::rstest]
#[tokio::test]
async fn unknown_ids_are_none(#[values(0, 2, u32::MAX)] id: u32) {
    assert_eq!(ItemStore::default().get(id).await, None);
}
```

## Fixtures

A `#[fixture]` function is injected by argument name and runs fresh for every test, which keeps cases independent.

```rust
#[rstest::fixture]
fn seeded() -> ItemStore {
    ItemStore::with(&[(7, "widget")])
}

#[rstest::rstest]
#[case::exact(7, Some("widget"))]
#[case::neighbour(6, None)]
#[tokio::test]
async fn seeded_lookup(seeded: ItemStore, #[case] id: u32, #[case] expected: Option<&str>) {
    assert_eq!(seeded.get(id).await.as_deref(), expected);
}
```

`#[once]` fixtures are shared statics: the test receives `&T`, the value is never dropped, and rstest forbids `#[once]`
on `async` and generic fixtures `[derived]`. nextest runs each test in its own process `[zip]`, so a `#[once]` value
is built once **per test process**: under nextest it saves no work across tests. Use it only for expensive, immutable
setup, and never for state a test mutates.

## Choosing another tool

| Situation | Use instead |
|---|---|
| Expected values come from the legacy TypeScript implementation | `parity_` fixtures (`parity-harness.md`) |
| The rule is the same for every input, or you want "all" inputs | a `prop_` test (`property.md`) |
| Inputs are files in a growing directory | datatest-stable's runtime discovery: rstest's `#[files]` resolves its glob at compile time, so new files are missed until a rebuild `[derived]` |
| Sequences of operations on stateful code or an API | a `model_` test (`model-based.md`) |
| Large plain output reviewed by eye (JSON, text, a value's structure) | an insta snapshot, below |

## Snapshots (insta)

Here insta is for plain values: JSON and text snapshots. Snapshots of rendered Dioxus markup belong to
`rust-test-app`. For large outputs: `insta = { version = "1", features = ["json"] }` and
`insta::assert_json_snapshot!(value)` `[zip]`.
A new or changed snapshot is written as a `.snap.new` file and reviewed with `cargo insta review`
`[unverified: cargo-insta not installed]` (installing `cargo-insta` is the user's decision: propose and wait).

- Inspect every snapshot before accepting it: accepting is a claim that the output is correct.
- A failing snapshot is a finding like any test: do not accept it to get green.
- In CI, insta defaults to never writing snapshots `[zip]`.
- Snapshot only stable output: normalise ids and timestamps first.
- Behaviour that must match TypeScript belongs in `parity_` fixtures, not snapshots.

## Rules

- One behaviour per test function; cases vary only the data.
- Assert the full expected value, not `is_ok()` or "contains".
- No shared mutable state between cases; use temp dirs for files.
- No time or randomness in cases: inject a clock or a seeded RNG.
