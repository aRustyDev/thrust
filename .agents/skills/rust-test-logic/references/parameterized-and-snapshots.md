# Example cases and snapshots (rstest, insta)

Tags: `[re-run]` reproduced on this machine; `[zip]` recorded by the source's author, not re-run; `[derived]` read
from crate docs or source, not run; `[unverified: <why>]` not run.

rstest does two different jobs. A `#[values]` **matrix** over a small finite domain is exhaustive, which makes it
generative: it is the first choice whenever the domain can be enumerated (see Matrices). A `#[case]` **table** is
static, and it is for inputs fixed for a reason outside the test: **one named case for every bug fixed or
counterexample shrunk**, examples the spec (or Plane's own TypeScript tests) names, and nothing else. Its name or a comment
says which. Known boundaries and tricky values belong in the `prop_` strategy as weighted values (`property.md`),
so that every run hits them. A table proves only the cases listed, so it never stands alone: the generator for the
same behaviour comes first.

Add `rstest = "0.26"` to `[dev-dependencies]` `[zip]` (0.26.1 resolved `[re-run]`). Every case becomes its own test,
for example `tests::isqrt_examples::case_3_below_square` `[zip]`, so nextest reports, filters and reruns each case
separately. A `for` loop inside one test stops at the first failure and hides the rest.

## Named cases

```rust
use rstest::rstest;

#[rstest]
// spec: the examples in isqrt's doc comment
#[case::spec_below_square(15, 3)]
#[case::spec_square(16, 4)]
// regression: issue 482, overflow at the top of the range (prop_isqrt_bounds counterexample)
#[case::issue_482_max(u64::MAX, 4_294_967_295)]
fn isqrt_examples(#[case] n: u64, #[case] expected: u64) {
    assert_eq!(isqrt(n), expected);
}
```

`isqrt`, its doc comment and issue 482 are examples. Zero, one and the maximum are boundaries: they belong in the
`prop_` strategy as weighted values, and a case appears here only for the reason its label gives. Always name cases (`#[case::description(..)]`): the name is what shows up in a
failure. A regression case is `#[case::issue_482_trailing_space(..)]` with a one-line comment linking the issue; never
remove one without removing the behaviour it pins. A shrunk proptest counterexample becomes one of these.

Put expected errors in the table instead of using `#[should_panic]`:

```rust
#[rstest]
// spec: the three framing errors the protocol document lists
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
naming `[zip]`). Several `#[values]` arguments multiply: 5 x 5 values is 25 tests. A matrix is exhaustive only when
its axes are the **whole** domain: every variant of an enum (role × permission, state × event), never three
hand-picked ids. The expected value comes from a spec table or `match` the test owns, never from the code under
test. Write that `match` with no `_` arm, so a new variant fails to compile there. The `#[values(..)]` list is
still typed by hand and silently stops being exhaustive when a variant is added: guard it with a test that
compares it with a `const ALL: [T; N]`, or with an iterator over every variant (strum's `EnumIter`). Control: break
one arm of the implementation, and the matrix must fail. Past a few hundred combinations, or when an axis is unbounded, write a property over the same spec table
instead. The `id` example below samples an unbounded axis, so it is a pinned case in matrix syntax, not an
exhaustive matrix.

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
| The domain is small and finite (every variant of a few enums) | an exhaustive `#[values]` matrix (Matrices, below) |
| The rule is the same for every input of an unbounded domain | a `prop_` test (`property.md`) |
| Inputs are files in a growing directory | datatest-stable's runtime discovery: rstest's `#[files]` resolves its glob at compile time, so new files are missed until a rebuild `[derived]` |
| Sequences of operations on stateful code or an API | a `model_` test (`model-based.md`) |
| Large plain output reviewed by eye (JSON, text, a value's structure) | an insta snapshot, below |

## Snapshots (insta)

Here insta is for plain values: JSON and text snapshots. Snapshots of rendered Dioxus markup belong to
`rust-test-app`. For large outputs: `insta = { version = "1", features = ["json"] }` and
`insta::assert_json_snapshot!(value)` `[zip]`.
A new or changed snapshot is written as a `.snap.new` file and reviewed with `cargo insta review`
`[unverified: cargo-insta not installed]` (installing `cargo-insta` is the user's decision: propose and wait).

- A snapshot is a static case: pin it only for a regression or for output recorded from an independent oracle, and keep a generative test of the same behaviour beside it.
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
