# Golden fixtures

Status: **live** (editor-doc corpus) · Gate: `scripts/rs cargo nextest run`

## Catches

X11 (P), X12 (P). Compares Rust output with **committed** expected output that an independent implementation (the
real TypeScript or Python code) produced. Hermetic: the reference implementation is not needed at test time.

## Use when / Not when

Use when a reference implementation exists and its output can be recorded: editor-document conversions, sanitiser
output, anything ported from TypeScript. Not when the inputs must be generated fresh each run (that is
[differential](differential.md)), and never by regenerating the fixture to make a failing test pass
([Rule 3](../rules.md)).

## Name and location

Prefix `parity_` (runs in the default profile). New fixtures go in the crate's `tests/parity/fixtures/` and are
committed ([Rule 7](../rules.md)). The editor-doc spike keeps its corpus in `crates/thrust-editor-doc/tests/corpus/`,
where `tools/spike-0006/gen-all.sh` writes it; it is committed, which is what Rule 7 requires.

## Run

```sh
scripts/rs cargo nextest run -p thrust-editor-doc     # [verified 2026-10-19: 12 passed]
```

Regenerating the committed corpus needs Node and the pinned Plane checkout: `tools/spike-0006/setup-oracle.sh`, then
`tools/spike-0006/gen-all.sh` `[verified 2026-10-19: fixtures regenerate byte-identical]`.

## Required controls

[Mutated fixture](../evidence-and-controls.md#mutated-fixture), [non-vacuity](../evidence-and-controls.md#non-vacuity),
[corrupted input](../evidence-and-controls.md#corrupted-input).

## Owning skill

`.agents/skills/rust-test-logic/SKILL.md` (`.agents/skills/rust-test-logic/references/parity-harness.md`) for the
Rust harness. When the fixtures come from a TypeScript module being ported, `.agents/skills/ts-to-rust/SKILL.md`
owns them and its frozen suite wins.

## In-repo examples

In `crates/thrust-editor-doc/tests/`: `stage1.rs`, `stage2.rs`, `stage3.rs` (against
`crates/thrust-editor-doc/tests/corpus/corpus.json`), `sanitize.rs` (against
`crates/thrust-editor-doc/tests/corpus/xss-corpus.json`), and the controls in `controls.rs`.

## Target

Parity fixtures for every TypeScript unit ported (`ts-to-rust`); the v1 API corpus is a separate,
[differential](differential.md) concern.
