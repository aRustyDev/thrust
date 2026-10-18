# Sources: rust-test-logic

Adapted from maintainer-supplied material (`rust-testing-plan.zip` and `reference-workspace.zip`, created
2026-10-18; drafted 2026-10-18). The prose is rewritten, but some passages are verbatim or near-verbatim from the
archive's skills: the Strategies bullets in `property.md`, the `ts-oracle.mjs` protocol bullet in
`parity-harness.md`, and several code snippets (the rstest named-case and expected-error tables, the `arb_frame`
property block, the lockstep loop). The files in `references/examples/` differ from the archive only by the header
and the edits listed under "Edits to the sample files". The archive's author and environment are known only from its
README (a sandbox that blocked the Playwright driver download).

Licence basis: the maintainer (aRustyDev) supplied both source archives and authorises the adapted text and templates under AGPL-3.0-only, 2026-10-18. The archives themselves carry no licence file.

## Inputs

| Source | Used for |
|---|---|
| `rust-testing-plan.zip`: `.claude/skills/testing-parameterized/SKILL.md` | `references/parameterized-and-snapshots.md` |
| `rust-testing-plan.zip`: `.claude/skills/testing-property/SKILL.md` | `references/property.md` |
| `rust-testing-plan.zip`: `.claude/skills/testing-model-based/SKILL.md`, `references/kv_store_model.rs`, `references/api_model.rs` | `references/model-based.md`, `examples/kv_store_model.rs`, `examples/api_model.rs` |
| `rust-testing-plan.zip`: `.claude/skills/testing-parity/SKILL.md`, `assets/{parity.rs,oracle.rs,gen-fixtures.mjs,ts-oracle.mjs}` | `references/parity-harness.md` (cut to what `ts-to-rust` lacks), the four matching examples |
| `rust-testing-plan.zip`: `.config/nextest.toml` | `examples/nextest.toml`, `references/nextest-profiles.md` |
| `rust-testing-plan.zip`: `README.md`, `AGENTS.md` | validation claims (tagged `[zip]`) and context only; no rule or routing text copied |
| `reference-workspace.zip`: `crates/core/src/test_support.rs`, `crates/core/tests/common/mod.rs` | `examples/test_support.rs`, `examples/tests_common_mod.rs` |
| `reference-workspace.zip`: `crates/core/Cargo.toml`, `crates/api/Cargo.toml` | dev-dependency versions only (rstest 0.26, proptest 1, proptest-state-machine 0.9, datatest-stable 0.3, tower 0.5 with `util`) |
| Crate docs and source in the local cargo registry cache: proptest 1.12.0 (`config.rs`, `failure_persistence/file.rs`), rstest 0.26.1 (`#[once]`, `#[files]`), datatest-stable 0.3.3 (`lib.rs`, `runner.rs`), proptest-state-machine 0.9.0 (`strategy.rs`, `test_runner.rs`) | `[derived]` claims |
| `cargo nextest run --help` (0.9.140) | flag names `--ignore-default-filter` and `--flaky-result`, default config path `[derived]` |
| In this repo: the `ts-to-rust` skill (`SKILL.md`, `references/freeze-and-oracle.md`, `json-fidelity.md`, `decision-table.md`, `type-map.md`) | read to remove duplication; linked by name, not edited |

## Dropped from the zip, and why

| Dropped | Why |
|---|---|
| Parity workflow, divergence triage, the JS-vs-Rust trap list (all but `toFixed` and the regex gap), the API-endpoint parity section, the `Math.round` port snippet, environment-recording and pinned-count advice | duplicated by `ts-to-rust` (freeze, controls, exception register, `json-fidelity.md`, recorded-response oracle) |
| `tests/parity/DIVERGENCES.md` and the hand-written `fixtures/intended/` directory | `ts-to-rust`'s exception register is the record, and fixtures are never hand-edited |
| `AGENTS.md` rules, routing table, definition of done, layout | repo-root material, deferred to the workspace PR; the ground rules in `SKILL.md` carry what this skill needs |
| "Done when" checklists | folded into the router's output contract |
| "AGENTS rule N" and `testing-xxx` cross-references | re-pointed to sibling skills by name or inlined |
| The toy `isqrt` / `KvStore` / `core_lib` source | not included; the examples import `your_crate` placeholders |

## Edits to the sample files

Each example carries a one-line `TEMPLATE` header with an origin line ("adapted from `<archive>`, 2026-10-18").
Renames: `core_lib` to `your_crate`, `crates/core` to `crates/your_crate` (`gen-fixtures.mjs`), `api` to `your_api`
(`api_model.rs`); `isqrt`, `KvStore` and `/api/items` stay as labelled example names. The `parity.rs` and
`gen-fixtures.mjs` headers also say "scalar units only". Four deliberate changes beyond that:

- The doc comment in `tests_common_mod.rs` carried the claim refuted under "Verification status"; it is rewritten to
  match the observed behaviour.
- In `nextest.toml` the `[[profile.default.overrides]]` block for `package(e2e)` is commented out: the archive's
  override retried every test in that package, including the hermetic harness test, contradicting "retries only for
  browser tests". The `e2e` profile and the `browser` test group are kept.
- In `nextest.toml` the comment above the `prop_|model_` override said a retry "still fails" the tests; it is
  rewritten because `--flaky-result pass` defeats the setting. The configuration lines are unchanged.
- In `gen-fixtures.mjs` the comment sending intended differences to `fixtures/intended/` now points at the
  `ts-to-rust` exception register.

## Verification status

Reproduced 2026-10-18 by the drafting session's lead on a scratch copy of the archive's workspace. Environment, stated
once: rustc and cargo 1.98.1, cargo-nextest 0.9.140, proptest 1.12.0 (rstest 0.26.1, datatest-stable 0.3.3,
proptest-state-machine 0.9.0), Node v26.9.0. Evidence is in the repo at
`.agents/plans/project-skills/verification/rust-testing-2026-10-18/` (`README.md` maps claims to logs; scripts in
`scripts/`, probes in `probes/`). Each row below cites its log in that directory's `logs/`.

| Claim group | Status and evidence |
|---|---|
| The archive's claim that default persistence silently drops counterexamples for files under `tests/` | claim in the archive, **refuted** `[re-run]`: the default prints `failed to find lib.rs or main.rs` twice, then `Saving this and future failures in <crate>/tests/x.proptest-regressions`, and the file is written; the `WithSource` helper writes the same file (`verify-4`, `verify-2` E2) |
| `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1` honoured by the default config (set: no file; unset: file) | `[re-run]` (`verify-4`) |
| A plain test that fails once then passes is FLAKY and exits 0; a `prop_` test under `--retries 2` or `NEXTEST_RETRIES=2` alone fails (exit 100) | `[re-run]` (`verify-2` E1) |
| `--retries 2 --flaky-result pass` and `NEXTEST_RETRIES=2 NEXTEST_FLAKY_RESULT=pass` make that `prop_` test pass (exit 0, `1 passed (1 flaky)`), so the per-test `flaky-result = "fail"` is a default, not a guard; a plain test under `--flaky-result fail` fails | `[re-run]` (`verify-9` T2) |
| `-E` filters intersect `default-filter` (no tests, exit 4); all six profiles resolve; `deep` at `PROPTEST_CASES=4096` passes; the default profile runs 31 tests (31 passed, 1 skipped) | `[re-run]` (`verify-2` E7, E0, E8; `verify-1-nextest`) |
| A fixture mutated in two places reports `2 divergence(s)` in one failure; `TZ=UTC node legacy-oracle/gen-fixtures.mjs` regenerates byte-identical fixtures | `[re-run]` (`verify-2` E4) |
| 256 oracle cases through one persistent Node process took about 0.1 s; a broken legacy function makes `oracle_` fail and shrink | `[re-run]` (`verify-2` E5) |
| `toFixed` vs `format!` at 0.5, 2.5, 0.125 (differ) and 0.375 (same); `Math.round` ties against `f64::round` | `[re-run]` (`verify-9` T1) |
| `toFixed` rounds ties by magnitude (negative ties mirror); the even-neighbour rule for when JS and Rust differ | `[derived]` reasoning from the re-run rows; negative ties were not run |
| `with_cases(n)` overriding `PROPTEST_CASES`; regex lookbehind; the `model` crate's age; the API id-reuse finding; the `is_empty` invariant story; insta's CI default; `#[once]` per process under nextest | `[zip]` |
| rstest `#[files]` is compile-time; `#[once]` limits; datatest-stable root, pattern, signatures and the panic on no match; proptest-state-machine hook order and rejection-based preconditions; presence-only `PROPTEST_DISABLE_FAILURE_PERSISTENCE`; proptest's printed advice to copy the `cc` line on CI; `--ignore-default-filter` exists; `.config/nextest.toml` is the default path; the template `spawn()` panics rather than skips without Node | `[derived]` |
| `cargo insta review` | `[unverified: cargo-insta not installed]` |
| `evals/evals.json` and `evals/triggers.json` | authored; **not run** against the final text; no with/without-skill delta measured |

## Open items

- The config cannot stop `--flaky-result pass` or `NEXTEST_FLAKY_RESULT=pass` from defeating a per-test
  `flaky-result = "fail"`. The ground rules forbid all four retry and flaky-result switches, and
  `nextest-profiles.md` asks for a CI step that fails if any appears (its control: add one on purpose). That step is
  not written or tested here.
- The archive's `parity.rs` fixture shape (`input` string, `expected` or `error` kind) differs from `ts-to-rust`'s
  lossless-encoder shape, and the error policy differs (kind or code versus message text). `parity-harness.md` says
  `ts-to-rust`'s format wins for ports it drives and scopes the templates to scalar units; neither skill was edited.
- Template limits found by reading (recorded in `parity-harness.md`, not patched in the examples): `oracle.rs`
  panics on any `{"error": ...}` and does not pin `TZ` or locale on the Node spawn.
- The `mutants` profile in `examples/nextest.toml` must stay identical to the copy in `rust-test-mutation`.

## Reference lifecycle

Re-check when proptest, rstest, datatest-stable or cargo-nextest ships a new minor or major, and replace `[zip]`
and `[derived]` tags with `[re-run]` as claims are reproduced. When the first Rust workspace lands, the examples
become live files: update the "template" wording and cite the live paths. When `ts-to-rust` changes, re-read
`parity-harness.md` for duplication. When `rust-test-fuzz`, `rust-test-mutation` and `rust-test-app` are installed,
confirm the mutual NOT clauses still point at the right names.
