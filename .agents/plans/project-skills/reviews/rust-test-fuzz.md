# Review: rust-test-fuzz (round 1)

Reviewer: skill-reviewer (fresh). Report transcribed by the drafting session's lead. Read every file in the draft; upstream
claims checked with WebFetch where possible; actionlint run on the CI template; spot-checks against the committed
verification pack (`.agents/plans/project-skills/verification/rust-testing-2026-10-18/`).

**Verdict: PASS (Section A clean). Grade: B.** Two MEDIUM findings recommended before install.

## Section A
No credential or config access, no fetched-content execution, no injection, no hidden text; author and version present; no
`allowed-tools` (pre-approves nothing); trigger specific; `sudo` appears only as "the user's decision"; the CI template is
least-privilege (`permissions: {}`, `persist-credentials: false`, pinned `--locked` install, target passed via env).

## Findings and disposition
1. MEDIUM: the cargo-fuzz command and its prose disagreed: `cargo fuzz run <t> fuzz/seeds/<t>` writes new inputs into the
   committed seeds (libFuzzer writes to the first corpus directory; cargo-fuzz passes its default corpus only when none is
   given). The archive's original has the same flaw. **Fixed**: `mkdir -p fuzz/corpus/<t>`, then `fuzz run <t> fuzz/corpus/<t>
   fuzz/seeds/<t>`; prose corrected; CI snippet updated; `[derived]`.
2. MEDIUM: SOURCES.md licence wording inadequate. **Fixed**: licence basis recorded per the maintainer's confirmation,
   2026-10-18; "adapted" wording.
3. LOW: `oom-`, `timeout-` and `leak-` artifacts unhandled (replaying an OOM input can exhaust memory; a leak cannot go red in the
   replay); no warning against seeding from production captures. **Fixed**.
4. LOW: the CI command omitted `-timeout=`. **Fixed**.
5. LOW: nextest assumed installed. **Fixed** (user's decision list; `cargo test fuzz_replay_` fallback).
6. LOW: boundary negatives untested (proptest "never panics", axum body); missing honggfuzz/bolero/`arbitrary`/`libfuzzer-sys`
   vocabulary. **Fixed**.
7. INFO: the `tmin` example stated `ff ff` as fact though the tool was not run. **Fixed** ("e.g.").
8. INFO: the evals were never run.

## Collisions
None with the installed skills. Mutual NOT clauses present. Drafts that overlap (parked, not installed): `rust-on-workers`
(wasm panic policy; eval 8 may route there) and `gha-workflow-design` (CI authoring; eval 10).

## Spot-checks: 8 `[re-run]` claims, all held
`--flaky-result pass` overrides the per-test `fail`; seeds removed with regressions present passes on the archive's original;
the archive's seeds are literal text (12, 15 and 8 bytes) so a codec break stays green on them and goes red on binary seeds;
`echo` writes 13 bytes under bash and `07 00 00 0a` under sh and zsh; the template gives 3 passes and 33 in the workspace and
controls F2 to F5 fail with the stated messages; the stale-mtime build; the four `printf` recipes match their hex; `xxd`'s second
argument is its output file. The reviewer also confirmed upstream: `tmin` syntax, the nightly requirement, artifact name
prefixes, libFuzzer defaults, `AFL_NO_CFG_FUZZING`, `system-config` using sudo, `fuzz!` aborting on panic, `afl` 0.18.x.

## Not verified
No fuzzing campaign could run: every cargo-fuzz and afl command, where `tmin` writes its output, afl platform and compiler
requirements, and the wasm "resets the instance" claim beyond the `[derived]` reasoning.

## After the review
The fixes were applied after this review and were **not re-reviewed by `skill-reviewer`**. The lead re-ran the mechanical gate and
confirmed the corpus-directory fix, the artifact-class guidance and the licence line by reading the changed text.
