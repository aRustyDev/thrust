# Status, gaps and drift

Measured on **2026-10-19**, on `main` at `991275e` plus the changes that resolved the drift below. Every claim names
the command that produced it; re-run them before relying on a number. Cargo commands go through `scripts/rs`
([Running](running.md)).

## Inventory

| Claim | Measured with | Result |
|---|---|---|
| Tests in the default nextest profile | `cargo nextest list --workspace` | **12**, all in `thrust-editor-doc` |
| Tests in the `oracle` profile | `cargo nextest list --workspace --profile oracle` | **4**, all in `thrust-editor-doc` |
| Tests with the default filter ignored | `cargo nextest list --workspace --ignore-default-filter` | **49**: 18 `contract`, 9 `worker_only`, 6 `pg_adapter`, 16 `thrust-editor-doc` |
| Default run | `cargo nextest run --workspace` | 81 passed (69 `thrust-core`, 12 `thrust-editor-doc`); `target/spike-0006/` not created |
| Mutation testing, `thrust-core` | `cargo mutants -f crates/thrust-core/src/lib.rs` | 57 mutants: 40 caught, **0 missed**, 17 unviable (each replaces a return value with `Default::default()`, which these types do not implement) |
| Oracle run, generated inputs missing | `cargo nextest run --workspace --profile oracle` | exit 100, **4 failed**, each naming its generator |
| Oracle run, inputs generated from Plane tag `oracle/v1-pin-20261018` | `setup-oracle.sh`, `gen-all.sh`, `tools/spike-0006/sanitize/run-all.sh`, `convert-sanitised.ts`, then the oracle run | exit 0, 4 passed: stage-3 fuzz 6000/6000 in all six directions, entities 6000/6000, sanitiser write path 389/392 (the 3 expected), 6000 mutated HTML inputs with 0 panics |
| Regenerating the committed corpus | `tools/spike-0006/gen-all.sh` then `git status` | fixtures byte-identical |
| Doctests | `cargo test --doc --workspace` | 0 in 10 crates |
| A crate with no tests of its own | `cargo nextest run -p thrust-contracts` | exit 4, `no tests to run` |
| In-source unit test modules | `grep -rlE '#\[cfg\(test\)\]\|mod tests' crates --include='*.rs'` | 1 file (`crates/thrust-core/src/lib.rs`) |
| Property/parameterized/fuzz/snapshot dev-dependencies | grep for `proptest`, `rstest`, `insta`, `datatest-stable`, `libfuzzer-sys`, `arbitrary` in every `Cargo.toml` | `proptest` and `rstest` in `thrust-core` only |
| Tests using a category prefix | `cargo nextest list` | 5 `prop_`, 1 `model_`, 3 `parity_`, 4 `oracle_`; no `fuzz_replay_`, `e2e_` |
| `fuzz/` or `tests/parity/` directories, proptest regression files | `find` | 1: `crates/thrust-core/proptest-regressions/lib.txt` |
| Planted mutants | grep `^mutant_` in `crates/thrust-adapters-*/Cargo.toml` | 4 |
| Architecture gate | `scripts/check-deps.sh` | exit 0 |
| CI | `.github/workflows/ci.yml`, first run on PR #28 | every job green; `gate` passed with all 16 planned cells reported |

Not re-run for this page (they need Docker, wrangler or a Cloudflare sandbox): `scripts/contract-all.sh`,
`scripts/mutants.sh`, `scripts/verify-all.sh`, `pg_adapter`, stage 2.

## Gaps

A gap is a technique or defect class the [strategy](strategy.md) needs and nothing provides yet.

| Id | Gap | Affects |
|---|---|---|
| G-1 | No skill owns the contract suite, real-backend tests, differential fuzzing against a JavaScript oracle, or the v1 oracle. Agents learn them only from these pages and the existing code. | [contract](techniques/contract-suite.md), [real backends](techniques/real-backends.md), [differential](techniques/differential.md) |
| G-2 | No skill owns planted mutants (`scripts/mutants.sh`). | [mutation](techniques/mutation.md) |
| G-3 | `thrust-core` has example, property and model-based tests and is mutation-tested; `thrust-contracts`, `thrust-api` and the adapters have none of their own, so cargo-mutants on them still exits 4 and their logic is tested only through the contract suite. | X15, [Rule 5](rules.md) |
| G-4 | Unowned defect classes: v1 API compatibility (X14), UI regression (X18), performance budgets (X19), migration up/down checks (X7), coverage thresholds (Q-0123.1), load and concurrency at scale (R-F2), SSRF and other security corpora (R-S3), cron behaviour beyond the sweeper. | [defect matrix](defect-matrix.md) |
| G-5 | CI does not run the `oracle_` tests, stage 2, the `blast` and `bench` contract passes or the probes, so those are checked only by hand. (`gate` is the required status check on `main` [verified 2026-10-19 fa4bd6c], so a red run blocks a merge.) | everything |
| G-6 | No wasm32 unit tests: the Worker is tested only over HTTP. | X8 |
| G-7 | No browser, desktop or mobile tests, and no `e2e` package or profile. | X18 |
| G-8 | Part built: `crates/thrust-contract-tests/tests/model/mod.rs` generates work-item create, delete and list sequences against a reference model and checks every caller × project × route for create, list, list with deleted, delete, project activity and attachment put, get and delete, in the `main` pass on every target. The sequences include attachments too (put, get, delete, of up to 5 MiB). The same model also covers events: outbox rows from creates and deletes, injected and redelivered events, sweeps and activity checkpoints, with the fast path on (`main`) and off (`sweeper`, `sweeper_model_outbox_delivers_what_was_swept_and_nothing_more`). Still hand-written only: rate limiting, the Worker's `cron` pass, and the Worker-only tests in `worker_only.rs`. [Rule 9](rules.md); ADR-0121. | [contract](techniques/contract-suite.md), [model-based](techniques/model-based.md) |
| G-9 | Part built: `crates/thrust-adapters-selfhosted/tests/model/mod.rs` generates `Database` and `Tx` call sequences (transactions that commit, roll back or are dropped, external-key conflicts, soft deletes, the outbox, activity, membership) against real Postgres and a reference model. Still not covered: concurrent transactions (the model is sequential; `concurrent_allocation_is_unique_contiguous_and_serialised` stays the only concurrency test), the memory adapter against the same model (it legitimately differs on rollback gaps), and migrations. | X6, X7, [real backends](techniques/real-backends.md) |
| G-10 | The editor-doc conversions have no hermetic generative test: the default loop checks only the committed corpus (`parity_`), and the generative `oracle_` runs are opt-in and not in CI (G-5). Target: Rust-only round-trip properties (for example JSON → Yjs → JSON is the identity on generated documents). | X11, X13, [golden fixtures](techniques/golden-fixtures.md) |
| G-11 | Only one generator's reach is checked: `model_strategy_reaches_every_step_class` covers the HTTP model, but no [generator coverage](evidence-and-controls.md#generator-coverage) control exists for the core `model_work_items_match_the_reference_model` or the `prop_` validators. | X16, every `prop_`/`model_` test |

## Drift

Places where the repo contradicts itself or its rules. Record a new item here with the next free id (D-18) and the
evidence; each item is one follow-up change.

| Id | Drift | Evidence |
|---|---|---|
| D-14 | A `worker-*` contract cell can fail because `wrangler dev` itself crashes mid-run, not because of the code under test: Rule 6 asks the CI loop to be deterministic, and this cell is not. Signature in the cell's `logs-<target>-<pass>` artifact (`*.server.log`): request times climb to 2-4 s, wrangler exits with an empty `✘ [ERROR]`, then the in-flight request gets a 500 from wrangler's proxy (absent from the Worker's own request log) and later ones `IncompleteMessage` or `ConnectionRefused`; the count guard then fails the cell (`passed=7 expected=15`). The `Broken pipe` lines and the `restarting build` line also appear in passing runs and are not the cause. Measured rate: 1 crash in 14 runs of `worker-cf main` (PR #28, `main` push run 39285650535, PR #31's two attempts, and 10 `flake-hunt` repetitions on `main` that reported `failures: 0 / 10`). Next steps: bump the pinned `wrangler` (4.123.0 in `.github/actions/setup-rust/action.yml`; the crashed run suggested 4.151.2) and re-measure with `flake-hunt`; the weekly `flake-hunt` schedule repeats only its default cell (`native-memory main`), so point it at `worker-cf main` to keep measuring. Until then, a crashed cell with this signature may be re-run once; any other failure is a finding (Rule 3). | PR #31 run 39317651310: attempt 1 crashed, attempt 2 on the same commit passed 15/15 and `gate` reported 16 of 16; `flake-hunt` run 39318238613 |
| D-15 | [Rule 9](rules.md) is newer than the tests. Static cases written before it do not say whether they are a regression, a spec example or oracle output, and the behaviours tested only by static cases are G-8 to G-10. Label or replace them when the file is next touched; a boundary case belongs in the generator as a weighted value. | `crates/thrust-core/src/lib.rs` `validate_name_cases` (9 unlabelled cases) and `validate_blob_name_cases` (12 of 13 unlabelled; `single_dot` is labelled) |

### Resolved

| Id | Drift | Resolution |
|---|---|---|
| D-1 | `AGENTS.md` said `cargo nextest run` selects nothing and exits 4; it ran 17 editor-doc tests. | `AGENTS.md` now says what the default profile runs and when exit 4 happens. |
| D-2 | `AGENTS.md` Rule 5 said the workspace's tests are all in `thrust-contract-tests`. | Reworded: most crates are tested only through it. |
| D-3 | Editor-doc tests carried no category prefix. | The golden tests over the committed corpus are `parity_*`; the tests that need the oracle's generated output are `oracle_*`. The corpus stays in `crates/thrust-editor-doc/tests/corpus/`, where the generator writes it; it is committed, which is all Rule 7 asks. |
| D-4 | Four tests passed while checking nothing when the git-ignored corpora were missing (the stage-3 fuzz test reported `D4 0/0`). | Renamed `oracle_*`, so they leave the hermetic default loop; each now fails when its input is missing and asserts it compared at least one input. |
| D-5 | `schema_cost.rs` was a timing printout with no assertion in the default loop. | Moved to `crates/thrust-editor-doc/examples/schema_cost.rs`. |
| D-6 | `corrupted_binaries_are_detected` wrote `target/spike-0006/corrupt.json` on every run. | Written only when `SPIKE0006_OUT=<dir>` is set. |
| D-7 | `contract.rs` cited a nonexistent `scripts/contract.sh mutants` pass. | Points at `scripts/mutants.sh`. |
| D-8 | `scripts/verify-all.sh` skipped the hermetic tests. | Step 0 runs `cargo nextest run --workspace`. |
| D-9 | `.cargo/mutants.toml` excluded files that do not exist, without saying why. | Comment explains they are excluded in advance for the skills' test-support files. |
| D-10 | The `oracle` and `deep` profiles selected nothing. | They select the 4 `oracle_` tests. |
| D-11 | `tools/spike-0006/setup-oracle.sh` did not build the editor's workspace dependencies, so `gen-all.sh` failed on a fresh checkout of the pin (`ERR_MODULE_NOT_FOUND`). | It builds them (`pnpm --filter "@plane/editor^..." run build`). |
| D-12 | `gen-all.sh` redirected straight into committed fixtures, so a failing oracle emptied them. | Writes to a temp file and moves it into place only on success with non-empty output. |
| D-13 | `run-oracle.sh` with no `tsx` installed ran `node ""`, printed nothing and exited 0. | Fails with a message naming `setup-oracle.sh`. |
| D-16 | `create_is_rate_limited_per_user` assumed its 31 creates land in one rate-limit window. Every limiter (memory, and the Worker's `RateLimiterDo`) is a fixed window aligned to multiples of its length, so a burst that straddles a boundary legitimately gets up to twice the limit: request 31 got `201` on `worker-cf main` (PR #67, run 39449013259), where the generated tests' extra load made each create take about 550 ms. | The test detects a crossing from its own clock (2 s of slack for skew) and repeats once with a fresh user, whose burst starts just after the boundary; a `201` inside one window still fails. `THRUST_EXPECT_RATE_WINDOW_SECS` (default 60) tells it the window. |
| D-17 | The `deep` profile selected tests by name prefix alone, so it also picked the contract suite's generated `model_` tests (added in PR #66), which panic without `THRUST_BASE_URL`, and would have picked `pg_adapter`'s, which need Docker: `cargo nextest list --profile deep` listed `thrust-contract-tests::contract model::model_work_items_over_http_match_the_reference_model`. | `default-filter` in `[profile.deep]` excludes `package(thrust-contract-tests)` and `binary(pg_adapter)`, like the default profile; the list no longer shows either. |
