# From crash to regression, and CI

Tags are defined in `SKILL.md`; engine commands and artifact paths are in `engines.md`.

## Crash to regression

1. **Find it.** cargo-fuzz saves `fuzz/artifacts/<target>/crash-<hash>`; afl.rs saves under
   `<out>/default/crashes/`. Do not commit that file as it is.
2. **Minimize.** `cargo +nightly fuzz tmin <target> <artifact>` `[zip]`
   `[unverified: nightly toolchain and cargo-fuzz not installed]`. Read its output for the path of the
   minimized file; do not assume one. Look at the bytes with `xxd` and judge whether the shrunk input is
   still the same bug.
3. **Name and place it.** Copy it to `fuzz/regressions/<target>/<short-description>` (`too-short`,
   `len-exceeds-buffer`): one file per bug, no hash, no colon. Never seed from production captures: they
   can put secrets or personal data into the repository. Craft inputs by hand or from the fuzzer.
4. **See it fail.** For a `crash-` artifact, `cargo nextest run -E 'test(/\bfuzz_replay_/)'` `[zip]` must go
   red and name the file. This is the proof that the regression file reproduces the bug without a fuzzer.
   `oom-`, `timeout-` and `leak-` artifacts differ (next list).
   - Stays green: the harness does not observe the crash. A `cfg(fuzzing)` bypass may be hiding the path
     (recompute the checksum in the input bytes), or the input was written as text (`harness-and-replay.md`,
     seed hygiene). Fix the input or the harness; do not weaken the replay.
   - Dies with a signal and no file name (stack overflow, abort, out-of-memory) `[derived]`: reproduce with
     the engine's single-input run, and do not raise a timeout or wrap it away.
   - `oom-` and `timeout-` artifacts `[derived]`: reproduce them through the engine first, bounded by
     `-rss_limit_mb` and `-timeout`. Replaying such an input under nextest can exhaust memory, and a hang is
     unbounded unless the nextest config sets a per-test limit (check that yours does). Commit the minimized
     input only after the fix makes it cheap.
   - `leak-` artifacts `[derived]`: a leak cannot turn the replay red, so "the replay must go red" holds only
     for crash-class artifacts. Keep a leak input as a regression for the engine run (give
     `fuzz/regressions/<target>` as a corpus directory after the generated one), not for the replay.
5. **Fix the code**, not the test. If the harness itself was wrong, say why in the commit message.
   Re-run the replay: green. Commit the fix and the regression file together.
6. **Generalize.** If the input exposes a class (any length field larger than the buffer), also add a named
   example for it and a `prop_` test for the class (`rust-test-logic`).
7. **If the decoder is a port** from another language, what the correct behaviour on that input is belongs to
   the port's frozen suite (`ts-to-rust`), not to the fuzzer.

Done when: every parser has a harness fn, a target and seeds; `fuzz_replay_<target>` is green with its
controls shown red; every crash is minimized, committed under `fuzz/regressions/`, fixed and covered.

## CI

- **Every PR:** the `fuzz_replay_` tests already run in the normal nextest job at no extra cost. Do not run
  the fuzzer on pull requests.
- **Scheduled:** one job per target on nightly Rust, `-max_total_time` of a few minutes and a small
  `-timeout`, a generated corpus directory ahead of the committed seeds, upload `fuzz/artifacts/` and the log
  when it fails, then follow the steps above.
- **Status handling:** the campaign is the last command of its step and its output is redirected to a log.
  Never end it with `; echo $?` or pipe it to `tee`: both replace the fuzzer's exit status, so a crash would
  report success.

A template, lint-checked with actionlint 1.7.12 (it rejects a deliberately broken copy), not run on GitHub.
The action pins are the ones already used in this repo's workflows; the installs run on a schedule you
approve, and a pinned cargo-fuzz version and nightly date are yours to choose.

```yaml
name: fuzz

on:
  schedule:
    - cron: "17 3 * * 1" # weekly; every PR already runs the fuzz_replay_ tests in the normal nextest job
  workflow_dispatch:

# Least privilege by default; the job raises what it needs.
permissions: {}

env:
  CARGO_FUZZ_VERSION: "0.0.0" # placeholder: pin the exact cargo-fuzz release you have reviewed
  CARGO_TERM_COLOR: never

jobs:
  fuzz:
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    permissions:
      contents: read
    strategy:
      fail-fast: false # one target's crash must not cancel the other targets
      matrix:
        include:
          - crate_dir: crates/your-crate # placeholder
            target: parse_frame # placeholder: one entry per fuzz target
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false

      - uses: dtolnay/rust-toolchain@683b588c487a69d01f6c6ddf40f036ff682cc2c8 # v1
        with:
          toolchain: nightly # once one works, pin a dated nightly (nightly-YYYY-MM-DD)

      - name: Install cargo-fuzz (exact version, locked)
        run: cargo install cargo-fuzz --locked --version "=${CARGO_FUZZ_VERSION}"

      # The campaign is the LAST command of its step and its output is redirected, so the step
      # status is the fuzzer's own. `cmd | tee log` or a trailing `; echo $?` would replace it.
      - name: Fuzz the target
        working-directory: ${{ matrix.crate_dir }}
        env:
          TARGET: ${{ matrix.target }}
        run: |
          mkdir -p "fuzz/corpus/${TARGET}"
          cargo +nightly fuzz run "${TARGET}" "fuzz/corpus/${TARGET}" "fuzz/seeds/${TARGET}" -- -max_total_time=300 -timeout=10 > fuzz.log 2>&1

      - name: Show the end of the log
        if: always()
        working-directory: ${{ matrix.crate_dir }}
        run: tail -n 60 fuzz.log

      - name: Upload crash artifacts and log
        if: failure()
        uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
        with:
          name: fuzz-${{ matrix.target }}
          path: |
            ${{ matrix.crate_dir }}/fuzz/artifacts/${{ matrix.target }}
            ${{ matrix.crate_dir }}/fuzz.log
          if-no-files-found: warn
          retention-days: 14
```

A campaign that finds a crash exits non-zero `[derived]`, which is what fails the step and triggers the
upload. The corpus directory is generated on the runner and never committed; a cached corpus between runs is
an optimization, not a requirement: the committed seeds are the starting set. The `-timeout`, corpus order
and `-max_total_time` choices are `[derived]` and not run.
