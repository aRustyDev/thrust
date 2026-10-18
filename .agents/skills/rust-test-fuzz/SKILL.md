---
name: rust-test-fuzz
description: >-
  Use when Rust code parses untrusted bytes or text and needs a fuzzer: cargo-fuzz, libFuzzer, `libfuzzer-sys`, afl.rs or cargo-afl, honggfuzz, bolero, `fuzz_targets/`, `cfg(fuzzing)`, `arbitrary` fuzz inputs, "decoder panics on bad input", a `fuzz/artifacts/crash-*` file to shrink with `cargo fuzz tmin`, "this parser must never panic on hostile bytes", or a `fuzz_replay_` or seed-corpus test that stays green with no inputs. NOT for proptest or round-trip properties (rust-test-logic), mutation testing (rust-test-mutation), axum, Dioxus or browser tests (rust-test-app), TypeScript parity (ts-to-rust), fuzzing Node/TS (jsfuzz, Jazzer.js), or pentests and CVE triage.
license: AGPL-3.0-only
author: aRustyDev
version: 0.1.0
---

# rust-test-fuzz

A fuzzer mutates inputs under coverage feedback and finds panics, aborts, hangs and failed assertions that no
example thought of. Campaigns run **outside** nextest. What nextest runs every time is `fuzz_replay_<target>`:
committed seeds and minimized crashes, replayed. A replay that is green with no inputs, or with seeds that
never reach the code, gates nothing.

## Ground rules
- Replay tests run with **cargo-nextest**; campaigns never do. Do not convert suites that parse
  `cargo test` output (`test result:`, exit 101) without migrating exit-code and pass-count handling
  (nextest exits 100 on failure `[derived]`). The replay is a plain `#[test]`: without nextest,
  `cargo test fuzz_replay_` runs it and libtest reports the result.
- nextest routes on the name prefix; this skill owns `fuzz_replay_`.
- No `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT`: a per-test
  `flaky-result = "fail"` is only a default, and `--flaky-result pass` overrides it `[re-run]`. A failure is a finding.
- A failing replay is a finding: never delete, weaken, `#[ignore]`, regenerate or lengthen its timeout.
- Hermetic, deterministic, inside the crate; commit `fuzz/seeds/` and `fuzz/regressions/`.
- Installing nextest, nightly, cargo-fuzz or cargo-afl is the user's decision: propose and wait.
- A check that cannot fail is not evidence. Seeds are **bytes**: write with `printf '\001\000'`, never
  `echo`; read back with `xxd <file>`, one file per call.

| Need | Read |
|---|---|
| Layout, harness rules, `cfg(fuzzing)`, replay and its controls, seed hygiene, wasm panics | `references/harness-and-replay.md` |
| cargo-fuzz and afl.rs commands, crash locations | `references/engines.md` |
| Crash to regression, scheduled CI | `references/crash-to-regression.md` |
| Harness and replay template, seed recipes | `references/examples/` |

## Procedure
1. Harness fn in `src/fuzz_harness.rs`: bytes in, never panics, a cheap invariant, returns whether it decoded.
2. Target wrapper in `fuzz/fuzz_targets/`; a valid and a rejected seed in `fuzz/seeds/<target>/`.
3. Add `fuzz_replay_<target>`; it must FAIL when it finds no inputs. Run
   `cargo nextest run -E 'test(/\bfuzz_replay_/)'` `[zip]`, then show its controls red: no `seeds/`, a broken codec.
4. Campaign, once the user has installed the engine.
5. Crash: `cargo fuzz tmin` `[unverified: nightly toolchain and cargo-fuzz not installed]`, copy to
   `fuzz/regressions/<target>/<name>`; the replay must go red.
6. Fix the code, commit fix and file together; a class of bug also gets a `prop_` test.
7. CI schedules campaigns; pull requests only replay.

## Output contract
Per target: harness, seed count, replay result with each control's failing message, campaign status (not
run / N seconds / crash and its regression path). Tags: `[re-run]` reproduced here;
`[zip]` recorded by the sample's author, not re-run; `[derived]` reasoned from docs or code;
`[unverified: <tool> not installed]` not run.

## Example
Q: "decode panics index out of bounds on a 2-byte body, ci left crash-9f2c". A: `tmin` (step 5) shrinks it to, e.g., `ff ff`. Copy to `fuzz/regressions/parse_frame/too-short`; the replay goes red naming it. Add the length
guard, replay green, commit both, add `prop_decode_never_panics` for the class.
