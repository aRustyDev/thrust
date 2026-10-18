# Sources: rust-test-fuzz

Adapted on 2026-10-18 from two archives kept at the repo root (`rust-testing-plan.zip` and
`reference-workspace.zip`; what is known of their author and environment is in their README). Neither is a
third-party skill under security review, so no do-not-reproduce list applies. Archive paths below are inside
the archives. Upstream pages were read, not reproduced.

Licence basis: the maintainer (aRustyDev) supplied both source archives and authorises the adapted text and templates under AGPL-3.0-only, 2026-10-18. The archives themselves carry no licence file.

Tags: `[re-run]` reproduced on 2026-10-18 by the drafting session's lead on a scratch copy of the archive's
workspace (rustc 1.98.1, cargo-nextest 0.9.140); the evidence is in
`.agents/plans/project-skills/verification/rust-testing-2026-10-18/` (its `README.md` maps claims to
`logs/verify-N.log`). `[zip]` recorded by the archive's author, not re-run. `[derived]` reasoned from
documentation or code, not executed. `[unverified: <tool> not installed]`.

## Archive files used

| Archive file | Used for | Disposition |
|---|---|---|
| `rust-testing-plan/.claude/skills/testing-fuzz/SKILL.md` | thesis, layout, cargo-fuzz and afl.rs commands, crash-to-regression steps, CI shape, done-when | adapted into the router and three references; sample names replaced; cross-references to its sibling skills removed or re-pointed |
| `.../testing-fuzz/references/fuzz_harness.rs` | harness plus replay test | adapted: every difference is listed under "What differs from the archive" |
| `rust-testing-plan/.config/nextest.toml` | how `fuzz_replay_` runs under the default profile (no override, `retries = 0`, default slow-timeout) | read for one paragraph; the file belongs to `rust-test-logic` |
| `rust-testing-plan/.cargo/mutants.toml` | the `**/fuzz_harness.rs` exclusion | one sentence; the file belongs to `rust-test-mutation` |
| `rust-testing-plan/AGENTS.md` | context; its rules rewritten into the ground-rules block | adapted |
| `reference-workspace/crates/core/Cargo.toml` | the `[lints.rust] unexpected_cfgs` entry | reproduced (four lines) |
| `reference-workspace/crates/core/src/lib.rs` | `#[cfg(any(test, fuzzing))] pub mod fuzz_harness;`, the proptest helper's call shape | pattern only |
| `reference-workspace/crates/core/src/frame.rs` | the code under test, used as the worked example | example name `frame` only |

Upstream pages fetched 2026-10-18 and read as summaries (a small model summarised each page; details the
summary did not state are listed under verification):
`rust-fuzz.github.io/book/cargo-fuzz/tutorial.html`, `llvm.org/docs/LibFuzzer.html`,
`github.com/rust-fuzz/cargo-fuzz`, `github.com/rust-fuzz/afl.rs`.

CI pins (`checkout`, `rust-toolchain`) are the ones in the installed `mdbook` skill's
`references/examples/docs.yml`; the `upload-artifact` pin is from the reviewed `gha-workflow-design` draft
under `.agents/plans/project-skills/`.

## What differs from the archive

- **Structure.** One 6.5 KB `SKILL.md` became a router of under 4 KB, three references, two examples,
  evals and this file.
- **Replay is stricter.** The archive asserted `replayed > 0` over seeds and regressions together and
  `continue`d on a missing directory. The template requires `seeds/` to exist and be non-empty on its own,
  requires at least one seed to reach the success branch (the harness fn now returns whether it decoded),
  sorts the files, and holds three permanent controls (empty directory, seeds that never decode, a lossy
  codec). It compiles unchanged as the archive's `crates/core/src/fuzz_harness.rs`; its three `fuzz_replay_`
  tests pass with the `seed-recipes.md` files in place, and each control fails with the named reason
  `[re-run]` (`verify-8`).
- **Corpus order.** The archive ran `cargo fuzz run <t> fuzz/seeds/<t>`, which would write new inputs into the
  committed seeds. The skill gives a generated, uncommitted `fuzz/corpus/<t>` first, seeds second, plus a
  small `-timeout` `[derived]`.
- **Added:** seed hygiene and the "must fail on zero inputs" rule; the `printf` recipe table; handling of
  `oom-`, `timeout-` and `leak-` artifacts; a no-production-captures warning; non-canonical-format
  invariants; the wasm abort note; the `cargo test fuzz_replay_` fallback; a note on honggfuzz and bolero;
  a widened retry rule (`--flaky-result` and `NEXTEST_FLAKY_RESULT` as well as retries); a hardened CI
  template (pinned actions, minimal permissions, status-preserving redirect).
- **Dropped:** the archive's advice to consult its parity skill about crashes in ported logic (reduced to
  one line pointing at `ts-to-rust`); its `.cargo/mutants.toml` and `.config/nextest.toml` bodies (other
  skills own them); its three committed fuzz inputs (below).

## What was not carried over, and why

**The archive's three committed fuzz inputs.** `crates/core/fuzz/seeds/parse_frame/empty-payload`,
`.../abc` and `crates/core/fuzz/regressions/parse_frame/crash-too-short` hold **literal escape text**, not
bytes: 12, 15 and 8 ASCII bytes (`5c 78 30 37 ...`) where `07 00 00`, `01 00 03 61 62 63` and `ff ff` were
meant `[re-run]`. None decodes, so every one lands on the length-mismatch path; `crash-too-short` is not even
shorter than 3 bytes. Measured on the archive's replay `[re-run]`:

| Case | Result | Log |
|---|---|---|
| original codec, shipped seeds | pass | `verify-2` E3 |
| codec broken (tag + 1), shipped seeds | **still pass**: the invariant never runs | `verify-2` E3d |
| codec broken, two correct binary seeds | fail: `decode -> encode must be lossless` | `verify-2` E3e |
| original codec, correct seeds | pass | `verify-3` (E3f redone; the first run was a stale build, see below) |
| `seeds/` and `regressions/` both removed | fail: `no fuzz inputs found` | `verify-2` E3b |
| only `seeds/` removed, `regressions/` present | **pass**: the summed count hides the gap | `verify-2` E3c |

The skill ships a `printf` recipe table (`examples/seed-recipes.md`) instead of the files.

## Verification status

| Claim group | Status |
|---|---|
| The defect and the table above | `[re-run]` (`verify-2` E3, `verify-3`) |
| The template compiles unchanged; its replay tests pass with the `printf` seeds; controls F2 to F5 each fail with the named reason; the seed bytes match `seed-recipes.md` | `[re-run]` (`verify-8`) |
| `xxd` takes one input file per call (a second argument is its output file and is overwritten) | `[re-run]` (`logs/verify-10-echo.log`; also the `s0-xxd` line in `verify-8`) |
| `echo "\007\000\000" > f`: bash writes 12 literal characters plus `0a`; zsh and `sh` write `07 00 00 0a`; `printf` writes `07 00 00` | `[re-run]` (`logs/verify-10-echo.log`) |
| After restoring an edited file from an archive, `touch` it or cargo reuses a stale build | `[re-run]` (`verify-3`) |
| `--flaky-result pass` and `NEXTEST_FLAKY_RESULT=pass` override a per-test `flaky-result = "fail"` | `[re-run]` (`verify-9`) |
| CI workflow in `crash-to-regression.md` | linted with actionlint 1.7.12, plus a must-fail control (a misspelled job key is rejected); not run on GitHub |
| nextest runs `fuzz_replay_` in the default profile with `retries = 0` | `[zip]` |
| every cargo-fuzz command, including the corpus order and `-timeout` | `[unverified: nightly toolchain and cargo-fuzz not installed]`; the corpus-order claim is also `[derived]` |
| every afl.rs command, the `afl = "0.18"` pin, the platform list | `[zip]`, `[unverified: cargo-afl not installed]` |
| `-max_total_time`, `-runs`, `-timeout` (default 1200), `-rss_limit_mb` (default 2048), `crash-`/`leak-`/`timeout-` names, nightly requirement, `AFL_NO_CFG_FUZZING`, `cargo afl system-config` needing root | `[derived]` from the fetched pages |
| Not stated by the fetched pages, so hedged in the text: where `tmin` writes its output, the default corpus directory, `fuzz/.gitignore` contents, `cargo fuzz run <artifact>` as the single-input reproduction, afl platforms and command lines, `oom-` file names, afl crash file naming, the behaviour of `oom-`/`timeout-`/`leak-` artifacts under the replay | `[derived]` or `[unverified]` as marked inline |
| Wasm panics abort on stable `wasm32-unknown-unknown` | `[derived]`, not run |
| Evals | authored; **not run**, so there is no with/without-skill delta |

## Load cost

`SKILL.md` is 4,086 bytes (about 1,020 tokens), paid on every invocation. The working references total
27,804 bytes (about 6,950 tokens) and load on demand: `harness-and-replay.md` 8.8 KB,
`crash-to-regression.md` 6.7 KB, `engines.md` 5.7 KB, `examples/fuzz_harness.rs` 4.8 KB,
`examples/seed-recipes.md` 1.9 KB. This file is provenance and is not loaded
at run time; neither are the evals. A typical task reads the router plus one or two references.

## Reference lifecycle

Re-check when cargo-fuzz, libFuzzer or afl.rs ships a major release, when the wasm toolchain gains stable
unwinding, or when this repo adopts its first nextest workspace (then replace `crates/core`-style paths with
the real crate and compile the template). When the nightly toolchain and cargo-fuzz are installed, run the
engine commands and replace each `[unverified]` tag. Re-tag `[zip]` claims as they are re-run.
