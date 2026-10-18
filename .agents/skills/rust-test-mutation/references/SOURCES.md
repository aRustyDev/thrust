# Sources: rust-test-mutation

Adapted from the archive's `testing-mutation` skill and its configs. The router and references are
rewritten and expanded, but they follow the archive skill's structure and carry its content: the command
table, the outcome table, the exit-code list and the six-step fixing procedure. These files differ from the
archive only by renames, a header line or comment edits:
- `examples/mutants.toml`: the archive's `.cargo/mutants.toml` with a header line, placeholders for the e2e
  crate directory and the error type, edited comments, and added commented-out `exclude_re` and
  `test_workspace` lines.
- `examples/profile-mutants.toml.txt`: the `[profile.mutants]` snippet from the reference workspace's
  `Cargo.toml` plus a header line.
- The inline `[profile.mutants]` block and the `default-filter` line in `references/interactions-and-ci.md`:
  from the archive's `.config/nextest.toml`, comments shortened.

Licence basis: the maintainer (aRustyDev) supplied both source archives and authorises the adapted text and templates under AGPL-3.0-only, 2026-10-18. The archives themselves carry no licence file.

All retrieved or read 2026-10-18.

## Verification environment

`[re-run]` means: reproduced 2026-10-18 by the drafting session's lead on a scratch copy of the archive's
reference workspace, with cargo-mutants 27.1.0 and cargo-nextest 0.9.140 (also rustc and cargo 1.98.1,
proptest 1.12.0, macOS). Evidence is in `.agents/plans/project-skills/verification/rust-testing-2026-10-18/`:
its `README.md` maps claims to `logs/verify-N.log`, and the scripts are in `scripts/`. The archive claims
cargo-mutants 27.1.0 and cargo-nextest 0.9.148.

## Inputs

| Source | Used for | Notes |
|---|---|---|
| `rust-testing-plan.zip`: `rust-testing-plan/.claude/skills/testing-mutation/SKILL.md` | commands, outcomes, exit codes, fixing procedure, interactions, CI | the maintainer's own material |
| `rust-testing-plan.zip`: `.cargo/mutants.toml`, `.config/nextest.toml` (the `[profile.mutants]` block and the `default-filter` line only) | `examples/mutants.toml`, the inline nextest block | the full `nextest.toml` is owned by `rust-test-logic` |
| `rust-testing-plan.zip`: `README.md`, `AGENTS.md` | validation claims (18/18, the `--in-place` finding), definition of done | read for context; no rule text copied |
| `reference-workspace.zip`: `Cargo.toml`, `crates/core/Cargo.toml`, `crates/core/src/store.rs`, `crates/core/tests/model.rs` | `examples/profile-mutants.toml.txt`, the `mutants = "0.0.3"` regular dependency, the worked example (two `is_empty` mutants) | toy names shortened in the router |
| cargo-mutants book, https://mutants.rs (pages: exit-codes, in-diff, pr-diff, iterate, in-place, nextest, skip, attrs, filter_mutants, skip_files, mutants-out, workspaces, timeouts, shards, baseline, lints, macros, limitations, cargo-args, error-values, ci, cautions, mutants) | facts tagged `[derived]`: exit codes 1/3/5/6/70, `--in-diff` and `--iterate` semantics, shard rules, `cap_lints` trade-off, `--no-config` | MIT (sourcefrog/cargo-mutants); facts and short snippets only |
| cargo-mutants `src/config.rs` on `main` (read through a page summariser) | key names `test_workspace`, `cap_lints`, `profile`, `exclude_re`, `exclude_globs`, `error_values`, `additional_cargo_test_args` exist | `[derived]`; not checked against the 27.1.0 tag |
| https://crates.io/api/v1/crates/mutants | newest `mutants` crate version | newest 0.0.4, MIT, read 2026-10-18 |

## Dropped from the archive, and why

- "AGENTS rule 5" and "see testing-e2e" cross-references: replaced by the fact itself (keep a crate's tests in
  that crate) or by the sibling skill's name.
- The `taiki-e/install-action` pointer: installing is the user's decision; the CI section says so.
- The `/tmp/pr.diff` path: replaced by `<scratch>`, outside the tree.
- The "Done when" checklist: folded into the output contract.
- Toy names (`core_lib`, `crates/core`, `KvStore`): example names only, except in quoted observed output.
- The archive README's claim that default proptest persistence loses counterexamples in `tests/`: not
  repeated; `verify-4` refutes it (it warns, but still saves beside the test file). That topic belongs to
  `rust-test-logic`.
- Added beyond the archive: `exclude_re` and `test_workspace` lines in `examples/mutants.toml` (key names
  `[derived]`), the `--in-diff` exit codes 5 and 6, the equivalent-mutant guidance, the dependency trade-off
  for `#[mutants::skip]`, the stale-mtime note, the list-twice check for `exclude_globs`, the
  `slow-timeout` inheritance note, the untracked-file caveat for `git diff HEAD`.

## Contradictions and corrections found

- A summarised copy of the cargo-mutants book page said the per-mutant logs are in `logs/`; the archive says
  `log/`, and cargo-mutants 27.1.0 writes `log/` `[re-run]` (`verify-6`). The references use `log/`. No rename
  is claimed; the cause of the mismatch is not established, so `ls mutants.out` once on a new version.
- The archive README says "add `/mutants.out*` to `.gitignore`"; this repo's `.gitignore` already has
  `**/mutants.out*/` (lines 12 and 14 of the worktree copy), so nothing is added.
- The archive says the shared proptest config helpers honour `PROPTEST_DISABLE_FAILURE_PERSISTENCE`.
  Reproduced with proptest 1.12.0 (`verify-4`): the default config honours it too, so it protects every
  property test; the reference says so.
- The archive's skill says one model-test invariant fixed the `is_empty` mutants. Re-run (`verify-5` M3a to
  M3c): either of two `is_empty` assertions (state-machine or lockstep) alone suffices; with both removed,
  2 MISSED, exit 2.
- The archive's `.cargo/mutants.toml` comment says the `mutants` profile excludes e2e and oracle tests; the
  exclusion actually comes from the `default-filter` in `[profile.default]`, which the profile inherits
  (`verify-5` M7 printed `1 test skipped via profile.default.default-filter`). The reference names the filter
  and gives a check with a must-fail control.
- The archive's `exclude_globs` lists four patterns; re-run (`verify-6`) showed two do the work (the e2e crate
  and `fuzz_harness.rs`) and `test_support.rs` and `tests/common/` never appear even without the config.
- The book's PR workflow diffs `origin/<base>..`; the archive uses `origin/main...` (merge-base). The
  archive's form is kept `[zip]`.

## Verification status

| Claim group | Status and evidence |
|---|---|
| `--list -f` on one file lists 18 mutants; a default run prints `18 mutants tested in 19s: 18 caught`, exit 0, `caught.txt` 18 lines, other outcome files empty | `[re-run]`, `verify-5` M1, M2 |
| The nextest invocations cargo-mutants makes, produced by `test_tool`, `additional_cargo_test_args` and `profile` | `[re-run]`, `verify-5` M2 (baseline log) |
| `mutants.out/` contents, `log/` singular, per-mutant file names | `[re-run]`, `verify-6` |
| `--in-place` trap: regressions file appears without the env var and not with it; source restored byte-identical | `[re-run]`, `verify-5` M5 |
| Either `is_empty` assertion kills both mutants; both removed gives 2 MISSED, exit 2 | `[re-run]`, `verify-5` M3a to M3c |
| `--iterate` re-tests only the 2 missed of 18 | `[re-run]`, `verify-5` M4 |
| `--in-diff` with a hand-made diff: 3 mutants listed, 3 caught; `diff -u -L` form | `[re-run]`, `verify-5` M6 |
| Exit codes 0, 2 and 4 (with the baseline error text); nextest `result: Failure(100)`; `terminate = "immediate"` stops siblings with SIGTERM then SIGKILL; the inherited `default-filter` | `[re-run]`, `verify-5` M2, M3c, M7 |
| `exclude_globs`: with the config, no e2e, harness, support or `tests/common` file listed; without it, the e2e crate and `fuzz_harness.rs` listed | `[re-run]`, `verify-6` |
| Default-config proptest: no regressions file with `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1`, one beside the test file without it | `[re-run]`, `verify-4` |
| Stale mtime after restoring a file from an archive makes a run use a stale binary (cargo and nextest, not through cargo-mutants) | `[re-run]`, `verify-3` |
| A per-test `flaky-result = "fail"` is only a default; `--flaky-result pass` or `NEXTEST_FLAKY_RESULT` overrides it (nextest, not cargo-mutants) | `[re-run]`, `verify-9` |
| Exit codes 1, 3, 5, 6, 70, `--iterate` file names, `--shard`, `--baseline`, timeouts, `cap_lints`, macro and `cfg` limits, `#[mutants::skip]` dependency rule, `exclude_re` matching, `--no-config`, `git diff HEAD` omitting untracked files | `[derived]`; not executed |
| Retries: a test that fails on a mutant and passes on retry is green, so the mutant is reported MISSED | `[derived]`; not executed |
| `slow-timeout` inherited from `[profile.default]` can kill a hung test before cargo-mutants' `--timeout` and report the mutant caught | `[derived]`; not run |
| Other commands (`-p`, `git diff` forms, `PROPTEST_CASES=64`), `error_values`, `test_workspace`, the sharded nightly line | `[zip]` or `[derived]`; not run |
| The inline nextest `mutants` block as written; the `cargo nextest list --profile mutants` check | the block is `[zip]` (its inheritance is `[re-run]` above); the check is `[derived]` and not run |
| CI snippets | shell fragments, not run; no workflow file is shipped |
| `evals/` | authored; not run against the final text; no with-skill versus without-skill delta measured. Eval 11 deliberately contains an imperative note as test data (an injection the agent must treat as data), so a reviewer reading it is not surprised |

## Reference lifecycle

Re-read the cargo-mutants book and re-check exit codes, `mutants.out/` names and config keys when
cargo-mutants moves past 27.1.0 or when the first workspace PR adds the live `.cargo/mutants.toml`. When
`rust-test-logic` publishes its full nextest config, diff its `[profile.mutants]` and `default-filter`
against `interactions-and-ci.md` and keep one of them authoritative. Upgrade a `[zip]` or `[derived]` tag to
`[re-run]` only after running the command with its must-fail control (a deliberately missed mutant, for
example both `is_empty` assertions removed).
