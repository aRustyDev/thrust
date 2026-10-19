# Status, gaps and drift

Measured on **2026-10-19 at `dd093fa`**. Every claim names the command that produced it; re-run them before
relying on a number. Cargo commands go through `scripts/rs` ([Running](running.md)).

## Inventory

| Claim | Measured with | Result |
|---|---|---|
| Tests in the default nextest profile | `cargo nextest list --workspace` | **17**, all in `thrust-editor-doc` |
| Tests with the default filter ignored | `cargo nextest list --workspace --ignore-default-filter` | **50**: 18 `contract`, 9 `worker_only`, 6 `pg_adapter`, 17 `thrust-editor-doc` |
| Default run | `cargo nextest run --workspace` | 17 passed, 3 reported leaky |
| Doctests | `cargo test --doc --workspace` | 0 in 10 crates |
| In-source unit test modules | `grep -rlE '#\[cfg\(test\)\]\|mod tests' crates --include='*.rs'` | 0 files |
| Property/parameterized/fuzz/snapshot dev-dependencies | grep for `proptest`, `rstest`, `insta`, `datatest-stable`, `libfuzzer-sys`, `arbitrary` in every `Cargo.toml` | 0 |
| Tests using a category prefix (`prop_`, `model_`, `parity_`, `oracle_`, `fuzz_replay_`, `e2e_`) | grep for `fn <prefix>_` under `crates/` | 0 |
| `fuzz/` or `tests/parity/` directories, `*.proptest-regressions` files | `find` | 0 |
| Planted mutants | grep `^mutant_` in `crates/thrust-adapters-*/Cargo.toml` | 4 |
| Architecture gate | `scripts/check-deps.sh` | exit 0 |
| CI workflows | `find .github -name '*.yml'` | 0 (`.github/` holds only `config/`) |

Not re-run for this page (they need Docker, wrangler, Node or a Cloudflare sandbox): `scripts/contract-all.sh`,
`scripts/mutants.sh`, `scripts/verify-all.sh`, `pg_adapter`, `tools/spike-0006/gen-all.sh`, stage 2.

## Gaps

A gap is a technique or defect class the [strategy](strategy.md) needs and nothing provides yet.

| Id | Gap | Affects |
|---|---|---|
| G-1 | No skill owns the contract suite, real-backend tests, differential fuzzing against a JavaScript oracle, or the v1 oracle. Agents learn them only from these pages and the existing code. | [contract](techniques/contract-suite.md), [real backends](techniques/real-backends.md), [differential](techniques/differential.md) |
| G-2 | No skill owns planted mutants (`scripts/mutants.sh`). | [mutation](techniques/mutation.md) |
| G-3 | No unit, property or model-based tests in any crate, so cargo-mutants has nothing to run (exit 4) and pure logic is tested only indirectly. | X15, [Rule 5](rules.md) |
| G-4 | Unowned defect classes: v1 API compatibility (X14), UI regression (X18), performance budgets (X19), migration up/down checks (X7), coverage thresholds (Q-0123.1), load and concurrency at scale (R-F2), SSRF and other security corpora (R-S3), cron behaviour beyond the sweeper. | [defect matrix](defect-matrix.md) |
| G-5 | No CI: every gate runs only when someone runs it by hand. The `ci` nextest profile's JUnit output has no consumer. | everything |
| G-6 | No wasm32 unit tests: the Worker is tested only over HTTP. | X8 |
| G-7 | No browser, desktop or mobile tests, and no `e2e` package or profile. | X18 |

## Drift

Places where the repo contradicts itself or its rules. Each item is one follow-up PR; this chapter does not fix them.

| Id | Drift | Evidence |
|---|---|---|
| D-1 | `AGENTS.md` §Testing says every test is a non-hermetic integration suite and that `cargo nextest run` selects nothing and exits 4. It selects 17 editor-doc tests and passes. | inventory rows 1 and 3 |
| D-2 | `AGENTS.md` Rule 5 says the workspace's tests are in `thrust-contract-tests`; `thrust-editor-doc` keeps its tests in its own crate. | `crates/thrust-editor-doc/tests/` |
| D-3 | The editor-doc tests use no category prefix (`entities_match_the_oracle` is a differential test that runs by default as an unprefixed test), and their fixtures are in `crates/thrust-editor-doc/tests/corpus/`, not `tests/parity/fixtures/` (Rules 4 and 7). | `cargo nextest list` |
| D-4 | Four editor-doc tests pass while checking nothing when the git-ignored corpora in `target/spike-0006/` are missing: `entities_match_the_oracle`, `mutated_html_never_panics` and `v1_write_with_sanitiser_matches_python_plus_js` print "no … corpus" and return; `stage3_fuzz_probe` finds the directory but no `fuzz-html*.json` files and reports `D4 0/0` (zero comparisons in every fidelity direction). Violates Rule 8. | `cargo nextest run -p thrust-editor-doc --no-capture` on a fresh worktree |
| D-5 | `schema_construction_cost` (`crates/thrust-editor-doc/tests/schema_cost.rs`) times with the wall clock and asserts nothing; it is a measurement in the default test loop (Rule 6). | source |
| D-6 | `corrupted_binaries_are_detected` (`crates/thrust-editor-doc/tests/controls.rs`) writes `target/spike-0006/corrupt.json`, outside a temp dir (Rule 6). Creating that directory is also what lets `stage3_fuzz_probe` (D-4) get past its "no corpus" check. | `ls target/spike-0006` after a run |
| D-7 | The header of `crates/thrust-contract-tests/tests/contract.rs` cites `scripts/contract.sh mutants`; no such pass exists. The mutation controls are `scripts/mutants.sh`. | `scripts/contract.sh` usage |
| D-8 | `scripts/verify-all.sh` does not run the `thrust-editor-doc` tests, so the umbrella gate skips the only hermetic suite. | its header |
| D-9 | `.cargo/mutants.toml` excludes `**/fuzz_harness.rs`, `**/test_support.rs` and `**/tests/common/**`, none of which exist. Harmless, but reads as coverage. | `find` |
| D-10 | `.config/nextest.toml` routes `oracle_` tests to the `ts-oracle` group and the `deep` and `oracle` profiles, but no test uses the prefix, so those profiles select nothing (exit 4). | inventory row 7 |
