# cargo-nextest and cargo-mutants

> **NOT YET USED IN THIS REPO.** Source: the nextest site (https://nexte.st) and the cargo-mutants
> book (https://mutants.rs, single-page edition), read 2026-10-18; crates.io lists cargo-nextest 0.9.148
> and cargo-mutants 27.1.0. Nextest behaviour marked `[re-run]` was observed with cargo-nextest 0.9.140
> in a scratch crate. cargo-mutants commands were not run by the skill author:
> `[unverified: cargo-mutants not installed]`. The skill never installs tools. Tag legend: `SKILL.md`.

## cargo-nextest

A test runner that replaces `cargo test` for unit and integration tests. Documented facts:

- **Process-per-test:** each test runs in its own process, which gives isolation and lets the runner
  kill, time, retry and capture each test. Cost: in-memory state cannot be shared between tests, and
  process creation is slower on Windows and sometimes macOS. (nexte.st "Why process-per-test?")
- **Doctests are not supported**: "Doctests are currently not supported because of limitations in
  stable Rust. For now, run doctests in a separate step with `cargo test --doc`." Keep that step
  (`doc-tests.md`).
- **Retries mark flaky tests:** `--retries N` retries a failing test up to N more times; a test that
  passes on a retry is marked **flaky**, and "by default, flaky tests are treated as ultimately
  successful": the run exits 0. Since **0.9.131**, `--flaky-result fail` (or `NEXTEST_FLAKY_RESULT=fail`)
  makes a flaky pass fail the run. Retries, backoff (`fixed`/`exponential`, `jitter`) and per-test
  `[[profile.default.overrides]] filter = 'test(..)'` live in `.config/nextest.toml`; the CLI flag and
  `NEXTEST_RETRIES` disable per-test overrides. **[re-run]** `--retries`, `--flaky-result`,
  `--stress-count` and `--stress-duration` are all listed by `cargo nextest run --help` in 0.9.140.
- **Stress runs:** since **0.9.103**, `--stress-count N` / `--stress-duration 90m` loops the selected
  tests; retries are retry-on-failure, stress is repeat-on-success.

```sh
cargo nextest run
cargo nextest run --stress-count 20 <test-name-filter>
cargo nextest run --retries 2 --flaky-result fail
```

### What swapping the runner into the scripts would break **[re-run]**

`scripts/contract.sh` and `scripts/mutants.sh` parse **libtest** output and exit codes. In a scratch
crate with two passing tests and one deliberate failure, using those scripts' own expressions:

| | `cargo test` | `cargo nextest run` |
|---|---|---|
| exit status on a failing test | 101 | **100** |
| `contract.sh` pass-count parse (`^test result: ... N passed`) | `2` | `?` (no match) |
| `mutants.sh` `... FAILED$` count | `1` | `1` (nextest echoes the failing test's captured output) |

Consequences: `mutants.sh` counts a mutant as KILLED only when the exit status is exactly **101**, so
under nextest **every killed mutant would be reported SURVIVED**; and `contract.sh` would read `?` for
the passed count, so every green run would trip the pass-count `VACUITY GUARD` (exit 5). Update both
parsers, then re-prove them with a mutant (`must-fail-controls.md`) before swapping.

- **Do not turn on retries without `--flaky-result fail`**: by default a flaky contract test exits 0,
  which is the failure `scripts/flake-hunt.sh` exists to expose.
- The contract tests are HTTP clients already isolated from the target they drive, so process-per-test
  adds little here **[derived]**. The likelier win is `--stress-count` as a quicker flake loop for one
  pass; it does not start the target (that stays `scripts/contract.sh`).

## cargo-mutants

An automatic mutation-testing tool: it "finds places where bugs can be inserted without causing any
tests to fail", by copying the source tree to a scratch directory, applying one machine-generated
mutation at a time, and running `cargo build` + `cargo test` (or nextest) on each.

- **Outcomes:** `caught` (a test failed), `missed` (no test failed: a gap, or an equivalent mutant),
  `unviable` (does not compile: inconclusive), `timeout`.
- **Exit codes:** 0 every viable mutant caught; 1 usage error; **2 some mutants missed**; 3 timeouts;
  **4 baseline tests already failing**; 5/6 bad `--in-diff`; 70 internal error. Machine-readable
  results are in `mutants.out/` (the repo's `.gitignore` already ignores `**/mutants.out*/`).
- **What it generates:** function bodies replaced by guessed values (unit, `0`/`1`/`-1`, defaults),
  binary and unary operator swaps, match arms and guards. It skips `#[cfg(test)]` code, test
  functions (including `#[tokio::test]`), `#[mutants::skip]` functions and `unsafe` functions.
- **Prerequisites (from the book):** "reliable non-flaky tests"; the baseline must pass **in the
  copied tree**; **"cross-compilation is not currently supported, so the tree must be buildable for the
  host platform."** Flaky tests make the results "meaningless".
- **Scope:** by default each mutant runs only the tests of the package being mutated; widen with
  `--test-workspace` or `--test-package`. `--package`, `--file`, `--list` select and preview.
- **Nextest:** `cargo mutants --test-tool=nextest` or `test_tool = "nextest"` in `.cargo/mutants.toml`.
  Nextest does not run doctests, so behaviour only doctests catch shows up as missed.
- **Cautions:** it is "as safe to run as cargo test", but tests with external side effects can be
  turned into bugs by a mutation; run in a disposable environment. `--in-place` edits the real tree
  (marker `/* ~ changed by cargo-mutants ~ */`); use a disposable checkout.

Commands (all `[unverified: cargo-mutants not installed]`):

```sh
cargo mutants --list                      # preview what would be mutated
cargo mutants -p thrust-adapters-memory   # one package
cargo mutants --test-tool=nextest
```

### Against `scripts/mutants.sh`

| | `scripts/mutants.sh` (hand-rolled) | cargo-mutants |
|---|---|---|
| Mutants | written by hand for named business rules (`cfg(feature)` pairs) | generated from the code |
| Oracle | the full HTTP contract suite against a started server (or one testcontainers test) | the package's own `cargo test` / nextest |
| Verdict | KILLED needs exit 101 **and** a `... FAILED` line; build/start failures are not kills | caught / missed / unviable / timeout; exit codes above |
| Targets | native targets only | host target only: **never the wasm32 Worker** |
| Strength | each mutant is chosen to break one specific rule and shows *which test* guards it | finds gaps nobody thought to write a mutant for |

They are complementary. **[derived]** Pointing cargo-mutants at `thrust-contract-tests` would fail its
baseline (exit 4): those tests need `THRUST_BASE_URL` and `THRUST_ADMIN_TOKEN` and a running target,
so the oracle that matters here cannot be its test command without a wrapper. It is better aimed at
crates whose tests are self-contained (pure crates, the adapter tests that start their own container,
which also carries the book's side-effect caution). Keep `scripts/mutants.sh` for "does the contract
suite guard rule X?"; use cargo-mutants to ask "what else is unguarded?". Either way, a `missed` mutant
is an assertion that observes nothing (`must-fail-controls.md`).
