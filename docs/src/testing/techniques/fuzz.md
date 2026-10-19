# Fuzz

Status: **live** as seeded mutation without a fuzzer (`decode_panics.rs`); coverage-guided fuzzing is
**template-only** · Gate: `scripts/rs cargo nextest run` (replay only)

## Catches

X13 (P), X11 and X12 (B). Feeds hostile or malformed bytes to code that reads outside input, looking for panics,
hangs and silent misdecoding.

## Use when / Not when

Use for every parser or decoder of bytes or text from outside the process: Yjs binaries, editor HTML, entity
decoding, request bodies. Pair it with a `prop_*_never_panics` [property](property.md) per class of crash found. Not
for round-trip correctness of trusted input (a property does that cheaper).

## Name and location

Harness in the crate's `src/fuzz_harness.rs` (excluded from mutation by `.cargo/mutants.toml`); seeds in
`fuzz/seeds/<target>/` written as real bytes, minimized crashes in `fuzz/regressions/<target>/`, both committed
([Rule 7](../rules.md)). A test named `fuzz_replay_<target>` replays them on every nextest run and must fail when
it finds no inputs. Campaigns (`cargo fuzz run`) run outside nextest, on a schedule.

## Run

```sh
scripts/rs cargo nextest run -E 'test(/\bfuzz_replay_/)'    # none exist yet [derived]
```

Campaign commands are in `rust-test-fuzz`; all are `[unverified]` there because no nightly toolchain was available.

## Required controls

[Empty input](../evidence-and-controls.md#empty-input) (an empty seeds directory must fail the replay),
[broken implementation](../evidence-and-controls.md#broken-implementation) (a codec that panics on one byte must be
caught by the committed seeds).

## Owning skill

`.agents/skills/rust-test-fuzz/SKILL.md`.

## In-repo examples

`crates/thrust-editor-doc/tests/decode_panics.rs`: `mutated_binaries_never_panic` mutates committed Yjs binaries
with a seeded RNG under `catch_unwind`; `mutated_html_never_panics` does the same for generated HTML but returns
early, passing, when its input file is missing (D-4). No `fuzz/` directory exists yet.

## Target

Coverage-guided fuzzing for the editor-document decoders and the sanitiser.
