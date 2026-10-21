# Property tests (proptest)

Tags: `[re-run]` reproduced on this machine; `[zip]` recorded by the source's author, not re-run; `[derived]` read
from crate docs or source, not run.

A property is a rule that holds for every input of a stated shape. proptest generates inputs and, on failure,
shrinks to a minimal counterexample. Name every property test `prop_*`: the shipped config gives those
`retries = 0` and `flaky-result = "fail"`, a default that `--flaky-result pass` defeats, so never pass it
(`nextest-profiles.md`).

## Finding properties

| Pattern | Example |
|---|---|
| Round-trip | `decode(&encode(&x)?) == Ok(x)`; `x.to_string().parse() == Ok(x)` |
| Defining relation | `r*r <= n < (r+1)*(r+1)` for `r = isqrt(n)` |
| Invariant | output is sorted; length preserved; balance never negative |
| Idempotence | `normalize(&normalize(x)) == normalize(x)` |
| Reference implementation | `fast(x) == naive(x)`; Rust vs legacy TypeScript (an `oracle_` test, `parity-harness.md`) |
| Metamorphic | adding a filter never returns more results; reordering input leaves a total unchanged |
| Robustness | `decode(any bytes)` returns `Ok` or `Err` and never panics |

Do not restate the implementation inside the test: a property that copies the code proves nothing. When a bug fix
suggests a whole class of inputs is affected, write the property as well as the named case.

## Config helpers

Two small files, both under `examples/`:

| Test lives in | Copy | Declare | Use |
|---|---|---|---|
| `src/` (unit tests) | `examples/test_support.rs` to `src/test_support.rs` | `#[cfg(test)] pub(crate) mod test_support;` | `#![proptest_config(crate::test_support::proptest_config(256))]` |
| `tests/` (integration) | `examples/tests_common_mod.rs` to `tests/common/mod.rs` | `mod common;` in each `tests/*.rs` | `#![proptest_config(common::proptest_config(256))]` |

Why a helper: `PROPTEST_CASES` (default 256) lets CI raise the case count and mutation runs lower it. A hardcoded
`ProptestConfig::with_cases(n)` overrides the variable `[zip]`, so a test that sets its own count ignores it. The
helper reads the variable and falls back to the test's own default.

### What the `tests/` helper buys, and what it does not

Observed on proptest 1.12.0 with the **default** config, in a failing `proptest!` under `tests/x.rs` `[re-run]`:
stderr prints `proptest: FileFailurePersistence::SourceParallel set, but failed to find lib.rs or main.rs` twice,
then `proptest: Saving this and future failures in <crate>/tests/x.proptest-regressions`, and the file **is**
written (`cc <hash> # shrinks to x = 10`). The `WithSource("proptest-regressions")` helper writes the same file in
the same place `[re-run]`. The default looks upward from the test file for a directory holding `lib.rs` or
`main.rs`; `src/` has one and `tests/` does not, so it falls back to a file beside the test `[derived]`.

So the helper gives you three things: no two-line warning on every failure, an explicit and stable location, and an
overridable `PROPTEST_CASES` default. It does not rescue counterexamples; the default does not lose them. If you see
the `failed to find lib.rs or main.rs` line, read the line after it for where the failure was saved. On a CI runner
that file dies with the runner: proptest then prints "you may wish to add the following line to your copy of the
file" `[derived]` followed by a `cc <hash>` seed line; copy that line into the committed regressions file, or turn the
shrunk input into a named case (below).

`PROPTEST_DISABLE_FAILURE_PERSISTENCE=1` is honoured by proptest's **default** config as well `[re-run]` (variable
set: no regressions file; unset: file written), so the helper's own branch for it is belt and braces, not a
requirement. proptest tests the variable's presence, not its value, so `=0` still disables saving `[derived]`. Use it
for runs that must not write into the source tree, such as mutation runs.

Where regressions live: `src/` tests write `proptest-regressions/<path>.txt` beside `src/` `[derived]`; `tests/`
tests write `tests/<file>.proptest-regressions`. Commit both.

## Writing the test

```rust
use proptest::prelude::*;

fn arb_frame() -> impl Strategy<Value = Frame> {
    (any::<u8>(), prop::collection::vec(any::<u8>(), 0..512))
        .prop_map(|(tag, payload)| Frame { tag, payload })
}

proptest! {
    #![proptest_config(crate::test_support::proptest_config(256))]   // tests/: common::proptest_config

    #[test]
    fn prop_frame_roundtrip(frame in arb_frame()) {
        let bytes = encode(&frame).unwrap();
        prop_assert_eq!(decode(&bytes), Ok(frame));
    }

    #[test]
    fn prop_decode_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        let _ = decode(&bytes);
    }
}
```

`Frame`, `encode` and `decode` are example names. Use `prop_assert!` / `prop_assert_eq!`: they report the failing
values cleanly. A panic also counts as a failure and is shrunk too.

## Strategies

- Basics: `any::<T>()`, ranges (`0u64..1_000`), regex strings (`"[a-z]{1,8}"`), `prop::collection::vec(s, 0..64)`,
  `prop_oneof![..]`, `.prop_map(..)`, `Just(x)`, `prop::sample::select(values)`, `prop_compose!` for multi-step
  generation.
- **Generate valid values by construction.** `prop_assume!` and `.prop_filter` reject cases, and proptest gives up
  when too many are rejected. Build the valid shape instead.
- **Use small domains where collisions matter** (keys from `"[a-d]"`, ids from `0..8`) so values repeat; bugs live
  in duplicates and overwrites.
- Give collections explicit size ranges; the default maximum is 100 elements.
- `proptest-derive`'s `#[derive(Arbitrary)]` is optional; a hand-written strategy usually documents the valid domain
  better.
- **Put the boundaries in the strategy, not in a table.** Empty, maximum length, the first invalid length,
  multi-byte characters and bare `.` go in as weighted alternatives (`prop_oneof![8 => general, 1 => Just(..)]`), so
  every run hits them. Named `#[case]`s are for pinned counterexamples and spec examples.

### Checking the strategy's reach

A property can pass because its strategy never produces the input that breaks the code. proptest has no
`classify`, so add an unprefixed test that samples the strategy with a fixed seed and asserts that every input class
the behaviour depends on appears `[re-run]` (proptest 1.12.0, in a scratch crate; a strategy missing one class
fails it). `blob_name`, `class_of` and `ALL_CLASSES` are placeholders for your own:

```rust
use proptest::strategy::{Strategy, ValueTree};

#[test]
fn blob_name_strategy_reaches_every_class() {
    const CASES: usize = 256; // the property's own case count: what one run draws
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let seen: std::collections::HashSet<_> =
        (0..CASES).map(|_| class_of(&blob_name().new_tree(&mut runner).unwrap().current())).collect();
    let missing: Vec<_> = ALL_CLASSES.iter().filter(|c| !seen.contains(c)).collect();
    assert!(missing.is_empty(), "never generated: {missing:?}");
}
```

`class_of` maps a value to a small enum the test owns (valid, empty, too long, contains `/`, a dot segment), and
`ALL_CLASSES` is a `const` array of its variants. For a `model_` test, classify whole sequences instead
(`model-based.md`, Rules).
`TestRunner::deterministic()` keeps the check stable, so it can sit in the default loop.

Sample what one run of the property draws, not more: a large sample only shows that the strategy reaches a class
eventually, not in a run. Measured in thrust-core: 1000 samples passed the old uniform strategies, 256 (their case
count) showed they never drew an empty name, a whitespace-only name or a one-character name `[re-run]`. A class
that one run misses goes into the strategy as a weighted value. If the property uses `prop_assume!`, also assert
the share of inputs it keeps.

## Async bodies

proptest bodies are synchronous. Build a current-thread runtime inside the body (needs `tokio` with `rt` as a
dev-dependency; keep it dev-only in a crate whose dependency policy forbids tokio at runtime):

```rust
use proptest::test_runner::TestCaseError;

let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
rt.block_on(async {
    // ... await calls, prop_assert_eq!(..) ...
    Ok::<(), TestCaseError>(())
})?;
```

## `PROPTEST_*` variables

| Variable | Effect |
|---|---|
| `PROPTEST_CASES` | case count (default 256); only a helper like the above lets a test's own count yield to it |
| `PROPTEST_VERBOSE=1` | print the cases |
| `PROPTEST_RNG_SEED` | fix the generator seed for the whole run |
| `PROPTEST_DISABLE_FAILURE_PERSISTENCE` | do not save or load failures (presence, not value) |

Other `PROPTEST_*` limits exist (rejects, shrink iterations) `[derived]`; an unknown `PROPTEST_*` name only prints a
warning.

## When a property fails

1. Read `minimal failing input:` in the output: that is the shrunk counterexample.
2. The seed is saved and replayed first on the next run. Locally, commit the regressions file. After a CI failure the
   file is gone with the runner: copy the printed `cc <hash>` line into the committed file (create it if needed).
3. Pin the shrunk input as a named rstest case (`parameterized-and-snapshots.md`). It reads better than a seed and
   survives changes to the generator.
4. Fix the code. Weaken the property only if the property itself was wrong, and say why in the commit message.
5. If TypeScript gives the same "wrong" answer, it is a parity question, not a bug: triage it with `ts-to-rust`
   before changing behaviour.
6. Never add retries to make a flaky property pass: the failure is the finding.

## Running

- `cargo nextest run` runs properties at their default case counts.
- `PROPTEST_CASES=4096 cargo nextest run --profile deep` is the long run (nightly CI) `[zip]`; `deep` allows 300 s
  periods per test.
- `cargo nextest run -E 'test(/\bprop_/)'` runs only properties `[zip]`.

## Rules

- The body is deterministic: no clock, no randomness, no network. All variation comes from the strategy.
- Keep each case fast, ideally under a millisecond: 256 cases of a slow body make a slow suite.
- Prefer proptest for new tests (explicit strategies, saved counterexamples). Keep existing quickcheck tests
  working and name them `prop_` too.
