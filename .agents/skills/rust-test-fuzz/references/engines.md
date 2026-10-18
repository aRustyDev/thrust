# Engines: cargo-fuzz and afl.rs

Both engines drive the same harness fn (`examples/fuzz_harness.rs`); seeds and regressions are shared.
Tags are defined in `SKILL.md`. No fuzzing campaign was run when this skill was written (see `SOURCES.md`),
so every campaign command below carries an `[unverified: <tool> not installed]` tag. Wording from the sample is
marked `[zip]`. If you do run one, say so in your report and drop the tag for that command only.

**Installing a toolchain or engine is the user's decision: propose it, wait for a yes, never run it
yourself.** That covers `rustup toolchain install nightly`, `cargo install cargo-fuzz`,
`cargo install cargo-afl` and `cargo afl system-config` (it needs root).

## cargo-fuzz (default)

libFuzzer in process: the fastest feedback and the default choice. It needs a nightly compiler "since it
uses some unstable command-line flags" (cargo-fuzz README, fetched). `fuzz/` is its own project and must
not join the main workspace (the main workspace stays on stable).

`[zip]` `[unverified: nightly toolchain and cargo-fuzz not installed]`

```sh
cd crates/<crate>
cargo +nightly fuzz init                 # once: creates fuzz/
cargo +nightly fuzz add <target>
```

`fuzz/Cargo.toml` needs its own `[workspace]` table; if it has none, add the directory to `exclude` in the
root `[workspace]` `[zip]`. `fuzz/fuzz_targets/<target>.rs`, a thin wrapper:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    <crate_name>::fuzz_harness::<harness_fn>(data); // trailing `;`: the target ignores the bool
});
```

Run a campaign: corpus directories follow the target name, and libFuzzer flags follow `--`. Give a
**generated, uncommitted** corpus directory first and the committed seeds second.

`[derived]` `[unverified: nightly toolchain and cargo-fuzz not installed]`

```sh
mkdir -p fuzz/corpus/<target>
cargo +nightly fuzz run <target> fuzz/corpus/<target> fuzz/seeds/<target> -- -max_total_time=600 -timeout=10
```

libFuzzer writes new inputs to the **first** corpus directory it is given, and cargo-fuzz supplies its default
`fuzz/corpus/<target>` only when no corpus is given (`[derived]` from the upstream source and docs, not run).
So `cargo fuzz run <target> fuzz/seeds/<target>` alone would write new inputs into the committed seeds;
the archive's original command had that flaw. `fuzz/corpus/` and `fuzz/artifacts/` are never committed: add
both to the crate's `.gitignore` (and check `fuzz/.gitignore` from `init` does). After a run, `git status`
must show `fuzz/seeds/` unchanged. Promote a useful new input to a seed deliberately: minimize it (`cargo fuzz
cmin` / `tmin`), write it with a descriptive name, and review it like code.

`-max_total_time` (seconds, default 0 = forever), `-runs`, `-timeout` (seconds, default 1200: an input that
takes longer counts as a failure, so set a small one for a parser) and `-rss_limit_mb` (default 2048) are
libFuzzer flags (LLVM libFuzzer docs, fetched).

## afl.rs (second engine, same harness)

Use it when cargo-fuzz plateaus or you want different mutation strategies on the same code. The sample
records AFL++ through afl.rs on x86-64 Linux and on macOS (x86-64 and ARM64), needing a C compiler and
`make` `[zip]`. A small bin crate (for example `crates/<crate>/fuzz-afl/`, also outside the main workspace)
depends on the library and `afl = "0.18"` `[zip]`; confirm the version on crates.io before pinning it.

`[zip]` `[unverified: cargo-afl not installed]`

```rust
fn main() {
    afl::fuzz!(|data: &[u8]| {
        <crate_name>::fuzz_harness::<harness_fn>(data);
    });
}
```

```sh
cargo afl build
cargo afl fuzz -i ../fuzz/seeds/<target> -o target/afl/<target> target/debug/<bin-name>
cargo afl run <bin-name> < target/afl/<target>/default/crashes/<file>      # reproduce one crash
```

`afl::fuzz!` turns panics into aborts so AFL records them as crashes `[zip]`. afl.rs builds with
`cfg(fuzzing)` unless `AFL_NO_CFG_FUZZING=1` is set (afl.rs README, fetched). `cargo afl system-config`
changes kernel settings through `sudo`: the user's decision.

## Other engines (honggfuzz, bolero)

No commands are given for them here. The harness fn and the replay are engine-neutral: call the same
`fuzz_harness` fn from their entry points, keep `fuzz/seeds/` and `fuzz/regressions/` as the shared inputs, and
follow that engine's own documentation for building and running `[derived]`.

## Where a crash lands

| Engine | Location | Note |
|---|---|---|
| cargo-fuzz | `fuzz/artifacts/<target>/crash-<hash>` | libFuzzer also writes `leak-<hash>` and `timeout-<hash>` files (libFuzzer docs, fetched) and `oom-<hash>` `[derived]`; see `crash-to-regression.md` for how those differ |
| afl.rs | `<out>/default/crashes/` `[zip]` | file names look like `id:000000,sig:06,...` `[derived]`; hangs go to a sibling `hangs/` directory `[derived]` |

Never commit a raw artifact name. Minimize, give it a short descriptive name with no hash and no colon
(colons break checkouts on some systems), and put it under `fuzz/regressions/<target>/`
(`crash-to-regression.md`).

## Reproduce a crash

1. With no fuzzer at all: copy the file into `fuzz/regressions/<target>/<name>` and run
   `cargo nextest run -E 'test(/\bfuzz_replay_/)'` `[zip]`. It must go red. This works on stable.
2. With the engine, one input: `cargo +nightly fuzz run <target> fuzz/artifacts/<target>/crash-<hash>`
   `[unverified: nightly toolchain and cargo-fuzz not installed]`.

A crash that kills the process (stack overflow, abort, out-of-memory) fails the nextest test with a signal
and does not name the file `[derived]`; use the engine's single-input run to see which input it was.
