# Commands, outcomes and `mutants.out/`

Tags: `[re-run]` reproduced with cargo-mutants 27.1.0 on the source's reference workspace (environment and
evidence per claim in `SOURCES.md`). `[zip]` recorded by the source's author, not re-run. `[derived]` read in the cargo-mutants
book (mutants.rs) on 2026-10-18, not executed. An untagged line is advice.

Before the first run: the tests pass under nextest (cargo-mutants runs a baseline first; when it fails the
run exits 4, below), and the templates in `examples/` are in place. The install
(`cargo install --locked cargo-mutants` `[zip]`) is the user's decision: propose it and wait.

## Commands

| Goal | Command |
|---|---|
| One file | `cargo mutants -f <path/to/file.rs>` `[re-run]` |
| One package | `cargo mutants -p <package>` `[zip]` |
| Only code changed on this branch | `git diff origin/main... > <scratch>/pr.diff` then `cargo mutants --in-diff <scratch>/pr.diff` `[zip]` |
| Uncommitted changes to tracked files | `git diff HEAD > <scratch>/wip.diff` then `cargo mutants --in-diff <scratch>/wip.diff` `[zip]` |
| A diff without git | `diff -u -L a/<path> -L b/<path> old new > <scratch>/pr.diff` `[re-run]` |
| Re-test only what was not caught last time | add `--iterate` `[re-run]` |
| Preview mutants without building | `cargo mutants --list -f <file>` `[re-run]` |
| Faster property tests during the run | `PROPTEST_CASES=64 cargo mutants ...` `[zip]` |

`git diff HEAD` omits untracked new files, so a brand-new file selects nothing `[derived]`. Either commit
first, or mark it with `git add -N <file>` (intent to add), which changes the user's index: that is the
user's decision, propose it and wait.

Keep `<scratch>` outside the source tree. Each mutant costs one incremental build plus one test run,
so scope runs while iterating and leave full runs to CI. Run `--list` first: an empty list means a
filter or an empty diff excluded everything, and a clean exit on zero mutants is not a pass.

Observed on one file of the reference workspace (names are that workspace's): `--list -f` listed 18
mutants; a default run printed `18 mutants tested in 19s: 18 caught` and exited 0; `caught.txt` had 18
lines and `missed.txt`, `unviable.txt` and `timeout.txt` were empty `[re-run]`.

`--in-diff`:
- It selects only mutants overlapping the regions the diff changes, and matches the diff against code
  under test, not test code: a diff that only edits tests selects nothing `[derived]`.
- Observed: a diff changing `self.entries.is_empty()` to `self.entries.len() == 0` made
  `--list --in-diff <file>` list 3 mutants (two return-value mutants and
  `crates/core/src/store.rs:37:28: replace == with != in KvStore::is_empty`); running them gave 3 caught
  `[re-run]`.
- Regenerate the diff after every edit. Exit 5 means the diff's new side no longer matches the tree;
  exit 6 means it is not a valid diff `[derived]`.
- It is a faster signal, not a substitute: run the whole module before calling it done `[zip]`.

`--iterate` reads `caught.txt`, `previously_caught.txt` and `unviable.txt` from the last `mutants.out`
(which becomes `mutants.out.old`) and skips what they list `[derived]`. It assumes your edits did not
reduce coverage, which may be false: finish with one run without `--iterate`. Observed after restoring
the two missing assertions: `Iteration excludes 16 previously caught or unviable mutants`, `Found 2
mutants to test`, `2 mutants tested ... 2 caught`, exit 0 `[re-run]`.

## Outcomes

| Outcome | Meaning | Action |
|---|---|---|
| caught | a test failed: good | none |
| missed (`MISSED`) | no test noticed the injected bug | fix: `fixing-missed.md` |
| unviable | the mutant did not compile | none (many of them in a workspace that denies warnings: `cap_lints`, see `interactions-and-ci.md`) |
| timeout | the mutant made the tests hang, often a loop condition | `interactions-and-ci.md`, then usually a justified skip |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | every viable mutant was caught `[re-run]` |
| 2 | some mutants were missed `[re-run]` |
| 3 | some tests timed out `[derived]` |
| 4 | the unmutated baseline fails: fix the tests first, this is not a mutation result. Output: `FAILED Unmutated baseline`, then `ERROR cargo test failed in an unmutated tree, so no mutants were tested`; the baseline log shows nextest's own `result: Failure(100)`, so nextest exits 100 when tests fail `[re-run]` |
| 1 | usage error `[derived]` |
| 5 / 6 | `--in-diff` diff does not match the tree / is not a valid diff `[derived]` |
| 70 | internal error `[derived]` |

**Read the exit code of cargo-mutants itself.** Redirect its output to a log (`cmd > log 2>&1`) and
make it the last command. A pipe to `tee` reports tee's status, and a trailing `; echo "exit=$?"`
reports echo's: both turn a failed run into exit 0. The counts are also in `mutants.out/outcomes.json`.

## `mutants.out/`

A run leaves `caught.txt`, `debug.log`, `diff/`, `lock.json`, `log/` (singular in 27.1.0), `missed.txt`,
`mutants.json`, `outcomes.json`, `timeout.txt` and `unviable.txt`. Per-mutant files in `diff/` and `log/` are
named like `crates__core__src__store.rs_line_37_col_9_001.diff` and `.log`; `log/` also holds the baseline
log `[re-run]`. The four `.txt` files list mutants by outcome, one per line as
`crates/core/src/store.rs:37:9: replace KvStore::is_empty -> bool with true` `[re-run]`. `outcomes.json`
holds the counts, `mutants.json` every generated mutant, and `--iterate` adds `previously_caught.txt`
`[derived]`.

This repo's `.gitignore` already ignores `**/mutants.out*/`, which also covers `mutants.out.old/`
(check: `grep -n mutants .gitignore`). Nothing to add.

## The `--in-place` trap

By default cargo-mutants works on a temporary copy of the tree. `--in-place` mutates the real source
files instead.

- Run in place without `PROPTEST_DISABLE_FAILURE_PERSISTENCE`, a regressions file appeared in the
  source tree (`crates/core/tests/model.proptest-regressions`, two `cc ...` seeds, one
  `# shrinks to ops = [Insert("a", 0)]` and one `# shrinks to ops = [Insert("a", 0), Get("a")]`, written
  from mutant counterexamples). With the variable set, no new file appeared. The mutated source file's
  sha1 was identical before and after, so cargo-mutants did restore it `[re-run]`.
- Restoring can still fail if the run is interrupted or panics `[derived]`. Do not edit, commit or run
  tests while it runs `[derived]`. If it happened: search the tree for `/* ~ changed by cargo-mutants ~ */`;
  run `git diff` on each affected file; remove ONLY the mutation hunk by hand; delete only the untracked
  regression files the run created (they come from mutants, not real bugs). **Never** `git checkout`,
  `git restore`, `git reset --hard` or `git clean` on files that may hold uncommitted edits: they destroy
  your work along with the mutation.
- It is incompatible with `--jobs` `[derived]`.

On a working checkout the answer is: do not use it. If the tree cannot be copied (size, absolute-path
dependencies), clone to a disposable directory first and run there. In a disposable checkout (CI),
and for any run that is not on a copy, set `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1`. proptest's default
config honours it too, so it covers every property test in the tree, with no shared config helper
needed (reproduced 2026-10-18 with proptest 1.12.0: a failing default-config property wrote no
regressions file with the variable set, and one beside the test file without it `[re-run]`). Still check
`git status` and `git diff` afterwards.

Mutated code runs your tests with machine-written changes, including to code that deletes or writes
files. That is why tests must be hermetic, and why a run belongs where a mistake is cheap
`[derived]`.

## A stale build looks like a result

After restoring a file whose mtime is older than the build (for example from an archive), cargo's mtime
fingerprint may skip the rebuild, and the next run uses a stale binary: a mutation experiment then
tests the wrong code. `touch` the file after restoring it. This was reproduced with cargo and nextest,
where it produced a false failing control `[re-run]`; it was not exercised through cargo-mutants.
cargo-mutants copies the tree, so also check that its first (baseline) run really rebuilt: read
`mutants.out/log/baseline.log`.
