# Harness and replay

Tags are defined in `SKILL.md`. `[re-run]` marks facts reproduced on the sample this skill derives from and
on `examples/fuzz_harness.rs`; `[zip]` marks that sample author's own claims, not repeated here.

## Layout

```text
crates/<crate>/
  Cargo.toml                  [lints.rust] check-cfg for cfg(fuzzing) (below)
  src/lib.rs                  #[cfg(any(test, fuzzing))] pub mod fuzz_harness;
  src/fuzz_harness.rs         harness fns, fuzz_replay_ tests and their controls (examples/fuzz_harness.rs)
  fuzz/                       cargo-fuzz project, its own workspace (needs nightly)
    fuzz_targets/<target>.rs  thin wrapper that calls the harness fn
    seeds/<target>/           small curated starting inputs, raw bytes (commit)
    regressions/<target>/     minimized crashing inputs, one file per bug (commit)
```

cargo-fuzz and cargo-afl build with `--cfg fuzzing`; nextest does not. So the harness module exists in fuzz
builds (`fuzzing`) and in unit tests (`test`) and never ships. Mutation testing should skip the file
(`**/fuzz_harness.rs` in the sample's mutation config `[zip]`); `rust-test-mutation` owns that config.

## Harness rules

- One function per target, raw bytes in, returning whether the input decoded (the replay uses that; the
  fuzz targets ignore it). A panic or failed assertion is a bug in the code under test, or in the harness:
  fix the harness only with the reason written in the commit message.
- Assert cheap invariants so the fuzzer checks correctness, not only crashes: decode then encode returns
  the input, length bounds hold, bad input is an `Err` and not a panic.
- Byte-for-byte round trip fits only a canonical format (one encoding per value). Where several encodings
  are valid (JSON whitespace, padding, field order) assert `decode(encode(decode(x))) == decode(x)`;
  byte equality would report bugs that are not bugs `[derived]`.
- Deterministic: no I/O, clock, threads or global state.
- A decoder that allocates whatever a length field declares is a finding, not a harness problem; libFuzzer
  reports out-of-memory and over-long inputs as failures too (`-rss_limit_mb`, `-timeout`).
- Structured input: derive `arbitrary::Arbitrary` behind a crate feature
  (`arbitrary = { version = "1", optional = true, features = ["derive"] }` `[zip]`) and keep the harness fn
  taking `&[u8]`, building the value inside with `arbitrary::Unstructured::new(data)`. One replay path then
  serves every engine `[derived]`.
- `#[cfg(fuzzing)]` in product code only bypasses checks that block exploration (checksums, signatures, rate
  limits), minimally. nextest builds without it: a crash found behind a bypass may not reproduce in the
  replay until the regression's bytes pass the real check (recompute the checksum in the input).

## `cfg(fuzzing)` and the check-cfg lint

Tell rustc the cfg is expected, or `#[cfg(fuzzing)]` warns `unexpected_cfgs`:

```toml
[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ["cfg(fuzzing)"] }
```

afl.rs sets `cfg(fuzzing)` by default too; `AFL_NO_CFG_FUZZING=1` turns that off (afl.rs README, fetched).

## The replay test

**KEY RULE: a replay or corpus test that returns early, or loops over zero files, when its input is missing
passes vacuously. It must FAIL when no inputs are found.** Not skip, not warn, not `continue`.

Routes to a vacuous pass:

- `read_dir` fails (directory missing, target renamed, `CARGO_MANIFEST_DIR` is a different crate) and the
  code `continue`s or returns.
- The directory exists and is empty.
- One count summed over seeds and regressions: a missing `seeds/` is hidden by a single regression file.
  Measured: removing `seeds/` while `regressions/` held a file still passed `[re-run]`.
- The files exist but the decoder rejects every one, so the invariant never runs (next section).

The shape that closes them (`examples/fuzz_harness.rs`):

1. `seeds/<target>` must exist and hold a file **on its own**. `regressions/<target>` may be absent or
   empty on a fresh target.
2. At least one seed must reach the success branch: fail if none decoded.
3. Sorted order; `catch_unwind` around each file with a message that names it.
4. One `fuzz_replay_<target>` test per target; the prefix is what nextest routes on.

How nextest treats it: in the sample's `nextest.toml`, `fuzz_replay_` has no override and the default filter
does not exclude it, so it runs in the default profile with `retries = 0` and the default `slow-timeout`
(30 s period, terminated after 4) `[zip]`. Everything committed under `seeds/` replays on every run, so keep
the corpus small and curated; never lengthen the timeout to fit a big corpus. The nextest config itself is
owned by `rust-test-logic`.

## Seed hygiene: raw bytes, checked

**Write seeds and regressions with `printf` octal escapes and read them back with `xxd` before you commit.**
A file holding the literal text `\x07\x00\x00` replays fine and checks almost nothing; nothing reports it.

Measured on the sample: its three committed inputs were literal text (12, 15 and 8 ASCII bytes where 3, 6
and 2 bytes were meant). None decoded. The replay stayed green with a codec deliberately broken so that
decode-then-encode was not lossless, and went red only after two correct binary seeds were added `[re-run]`.

`echo` is the usual cause. `echo "\007\000\000" > f` under bash writes the 12 literal characters plus a
newline (`5c30 3037 ... 0a`); under zsh and under macOS `sh` the escapes are interpreted and the file is
`07 00 00 0a`, a trailing newline byte that turns a valid length-prefixed frame into a length mismatch
`[re-run]`. `printf` is the only portable choice. Read each file back with `xxd <file>`, one file per call: a
second argument to `xxd` is its output file and overwrites it `[re-run]`.
Recipes and the expected hex: `examples/seed-recipes.md`. Keep one valid seed and one rejected seed per target.
Never seed from production captures: they can put secrets or personal data into the repository.

## Controls: the replay must be shown to fail

| Control | How | Must |
|---|---|---|
| Seeds missing or empty | with `regressions/<target>` still present, move `fuzz/seeds/<target>` aside and run the replay | go red: `no seed inputs found`. The template also holds this as a permanent test (empty directory) |
| Seeds never decode | the template's permanent test writes a text-escape file as the only seed | go red: no seed reaches the success branch |
| Invariant can fail | make `decode` lossy for a moment (drop a payload byte, or add 1 to the tag) | go red naming a seed: `decode -> encode must be lossless`. The template holds an in-test lossy codec for this |
| Regression is real | before the fix, the regression file is in place | go red naming that file |

Exercised on the template in a copy of the sample workspace `[re-run]`: it compiles unchanged and its three
`fuzz_replay_` tests pass (33 tests in the whole workspace). Each control then failed with the named reason
(exit 100): a broken codec gave `decode -> encode must be lossless` and `fuzz input <path> panicked`; `seeds/`
removed with `regressions/` present gave `no seed inputs found in <dir>` (the sample's own replay passes in
that case, so the stricter check is a real fix); a text-only seed gave `no seed in <dir> reaches the success
branch`; a one-byte regression input that makes the decoder panic failed naming that file. Restoring
everything passed.

The permanent tests prove the logic on every run; the manual controls prove the wiring (paths, target names).
After restoring an edited file, `touch` it: a file restored from an archive can keep an old mtime and cargo
may reuse the stale mutated build `[re-run]`.

## Keep `prop_<parser>_never_panics` beside it

A proptest that feeds arbitrary bytes to the same harness fn runs in the normal loop in milliseconds; the
fuzzer is the campaign (coverage-guided, minutes to hours). The `proptest_config` helper and the `prop_`
conventions belong to `rust-test-logic`.

```rust
proptest! {
    #![proptest_config(crate::test_support::proptest_config(256))]

    #[test]
    fn prop_parse_frame_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        crate::fuzz_harness::parse_frame(&bytes);
    }
}
```

`[derived]` from the sample's property test, not compiled here.

## Why "never panics" matters on wasm `[derived]`

On `wasm32-unknown-unknown` with the stable toolchain a panic aborts: there is no unwinding, so
`catch_unwind` cannot recover. A decoder that panics on hostile bytes inside a Worker therefore resets the
instance. Native tests and campaigns unwind, so they find the bug but understate its production cost. A panic
reachable from request bytes is an outage bug: return an `Err` instead of relying on a catch.
