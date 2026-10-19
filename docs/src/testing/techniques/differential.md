# Differential

Status: **live** (editor-doc spike, offline) · **planned** (`oracle_` tests, public v1 oracle) · Gate: none today

## Catches

X11 (B), X12 (P), X13 (B), X14 (planned primary). Runs the same inputs through Rust and through a reference
implementation and compares the outputs. Unlike [golden fixtures](golden-fixtures.md), the inputs can be generated or
mutated in bulk, so it finds disagreements nobody thought to record.

"Oracle" means three different things in this repo; read the [Glossary](../glossary.md#oracle) before using the word.

## Use when / Not when

Use when a reference implementation is runnable and the input space is too large to curate: HTML and Yjs conversion,
sanitiser output, entity decoding, and (planned) the whole v1 API against pinned Plane. Not as a PR gate while the
reference needs Node, pnpm or Docker; keep it in an opt-in profile or a scheduled run.

## Name and location

Live comparisons in nextest use prefix `oracle_` (excluded from the default profile, two at a time in the
`ts-oracle` group). The editor-doc spike instead generates its differential corpora offline into git-ignored
`target/spike-0006/` and replays them in ordinary tests (`stage3_fuzz.rs`, `entities_fuzz.rs`, `sanitize.rs`).
The generators and the reference side are in `tools/spike-0006/oracle/` and, for the Python `nh3` reference,
`tools/spike-0006/sanitize/`.

## Run

```sh
tools/spike-0006/gen-all.sh                                            # needs Node + pnpm [unverified: not run]
scripts/rs cargo nextest run -p thrust-editor-doc                      # replays what gen-all produced [verified 2026-10-19 dd093fa]
scripts/rs cargo nextest run --profile oracle                          # oracle_ tests; none exist yet [derived]
```

**Without the generated files the replay tests pass while comparing nothing** (measured: `stage3_fuzz_probe` reports
`D4 0/0`; three others print "no … corpus" and return). That is drift D-4 in [Status](../status.md); until it is
fixed, a green run of these four tests is not evidence unless `gen-all.sh` ran first.

## Required controls

[Non-vacuity](../evidence-and-controls.md#non-vacuity) (the replay must fail when it has no inputs),
[must-fail case](../evidence-and-controls.md#must-fail-case), [mutated fixture](../evidence-and-controls.md#mutated-fixture).

## Owning skill

`.agents/skills/rust-test-logic/SKILL.md` (`.agents/skills/rust-test-logic/references/parity-harness.md`) owns
`oracle_` tests against a Node process. Differential fuzzing against a JavaScript reference and the v1 API oracle have **no owning skill** (G-1).
`ts-to-rust` covers replaying recorded HTTP responses against a port.

## In-repo examples

`crates/thrust-editor-doc/tests/stage3_fuzz.rs`, `entities_fuzz.rs`, `sanitize.rs`
(`v1_write_with_sanitiser_matches_python_plus_js`); reverse direction (Rust output loaded by the real JavaScript)
in `tools/spike-0006/oracle/verify-rust-out.ts`.

## Target

Schema diff as the PR gate, a curated corpus and a nightly differential run against pinned Plane: ADR-0122
(`.agents/plans/contrail/adrs/test/patterns/0122-v1-oracle-and-parity-tests.md`).
