# Verification pack: the Rust testing method (2026-10-18)

Evidence behind every `[re-run]` tag in `rust-test-logic`, `rust-test-fuzz`, `rust-test-mutation` and
`rust-test-app`. Run by the lead of the drafting session (an AI session working on the maintainer's machine)
against `reference-workspace.zip`, extracted into a scratch directory. Nothing here ran inside the repo.

Environment: macOS (Apple silicon), rustc/cargo 1.98.1 (Homebrew), cargo-nextest 0.9.140 (the archive's config
recommends 0.9.148 and prints a notice; it requires 0.9.131), cargo-mutants 27.1.0, Node v26.9.0. Resolved
crates: proptest 1.12.0, proptest-state-machine 0.9.0, rstest 0.26.1, datatest-stable 0.3.3, dioxus and
dioxus-ssr 0.7.10, axum 0.8.9, tower 0.5.3, tower-http 0.7.1, reqwest 0.13.5. Not run: cargo-fuzz, cargo-afl and
any nightly toolchain (not available), Playwright, `dx`.

Paths in the scripts and logs are scrubbed to `$SCRATCH`, `$REPO`, `$WORKTREE` and `$HOME`; `Compiling` lines and
the nextest version banner were dropped from the logs. To re-run: extract the archive to `$SCRATCH/ref`, copy
`probes/` to `$SCRATCH/probes/`, set `SC`/`SCRATCH` in a script and run it. Every script writes each command's
exit status into its own log (`[label] exit=N`); the script's own exit code means nothing.

| Script and log | What it shows |
|---|---|
| `logs/verify-1-nextest.log` | default profile: 31 tests run, 31 passed, 1 skipped |
| `verify-2` | profile lists (E0); nextest retries: a plain test is FLAKY and exits 0, a `prop_` test under `--retries 2` or `NEXTEST_RETRIES=2` fails (E1); proptest persistence (E2); `fuzz_replay_` baseline and controls (E3); parity controls and deterministic regeneration (E4); oracle controls (E5); e2e controls (E6); `-E` filters intersect the profile's `default-filter`, empty selection exits 4 (E7); `deep` at 4096 cases (E8); doctests and `ci` profile (E9) |
| `verify-3` | E3f redone: the first E3f was a stale build (the archive restore kept an old mtime, so cargo did not rebuild); after `touch` the original codec passes |
| `verify-4` | proptest persistence isolated: default config in `tests/` prints `failed to find lib.rs or main.rs` twice but still saves the file; `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1` stops the file for the default config too. Refutes the archive's claim that counterexamples are silently dropped |
| `verify-5` | cargo-mutants 27.1.0: 18 mutants, 18 caught; `--in-place` trap; either `is_empty` assertion alone catches both mutants, both removed gives 2 MISSED exit 2; `--iterate`; `--in-diff`; baseline failure exits 4 |
| `verify-6` | `exclude_globs` control (with and without the config); `mutants.out/` layout (`log/` is singular) |
| `verify-7`, `verify-7b` | the `rust-test-app` templates compiled and run. **`verify-7`'s ui control (c1) was badly designed: it mutated production code and assertions identically, so it stayed green. `verify-7b` is the corrected control (production line only: 2 of 4 tests fail).** The e2e control in `verify-7` is valid and shows `TRY 1/2/3 FAIL` under the archive's retry override |
| `verify-8` | the `rust-test-fuzz` template compiled with `printf` seeds; controls F2 to F5 each fail with the named reason. (`s0-xxd` exits 1 only because `xxd` takes one input file; the seed bytes were dumped separately and match `seed-recipes.md`) |
| `verify-9` | JS vs Rust numeric facts (`probes/roundtrip.*`); `--flaky-result pass` and `NEXTEST_FLAKY_RESULT=pass` override a per-test `flaky-result = "fail"` |
| `verify-11` | the FINAL `rust-test-app` templates (after the review fixes): default profile and e2e profile pass; the strengthened ui assertions (`data-testid="entry-2" class="done"`) fail when the class branch is inverted in production code, while the same inversion on the archive's original template stays green (the review's finding, reproduced) |
| `verify-10-echo` | `echo "\007\000\000" > f`: bash writes the 12 literal characters plus `0a`; zsh and `sh` write `07 00 00 0a`; `printf` writes `07 00 00`; `xxd a b` treats `b` as its OUTPUT file and overwrites it |

`logs/verify-1-nextest.log` came from `cargo nextest run --manifest-path <extracted>/Cargo.toml --workspace`.

Findings about the archive itself, all reproduced here: its committed fuzz seeds are literal escape text, so its
replay stays green against a broken codec; its replay tolerates a missing `seeds/` directory when a regression
file exists; its README's persistence claim is wrong for proptest 1.12.0; its `package(e2e)` retry override also
retries the hermetic harness test; a per-test `flaky-result = "fail"` is only a default.
