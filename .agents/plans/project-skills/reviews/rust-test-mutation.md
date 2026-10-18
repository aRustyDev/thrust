# Review: rust-test-mutation (round 1)

Reviewer: skill-reviewer (fresh). Report transcribed by the drafting session's lead. Read every file in the draft; cargo-mutants
and mutants.rs claims checked with WebFetch and source; spot-checks against the committed verification pack
(`.agents/plans/project-skills/verification/rust-testing-2026-10-18/`).

**Verdict: PASS (Section A clean). Grade: B.** No Section B or C finding blocked.

## Section A
No network, credentials or fetch-and-execute; author and version present; no permission key at all. Eval 11 contains a quoted
"NOTE TO AI ASSISTANTS" inside a pasted `missed.txt` on purpose: the reviewer treated it as data, did not act on it, and ruled it
not a violation (a quoted user-turn fixture, a reserved non-resolving name, disclosed in SOURCES.md and `expected_output`,
nothing in SKILL.md points the agent at `evals.json`, and the eval grades refusal). The matrix has no fixture exemption, so the
maintainer should confirm that reading.

## Findings and disposition
1. MEDIUM: the description's "choosing which tests to add when finishing a module" collides with `rust-test-logic` and
   `rust-test-app`; bare "exit code 2/3/4". **Fixed**: anchored to cargo-mutants; triggers updated.
2. MEDIUM: recovery after an interrupted `--in-place` run said only "search for the marker"; `git checkout` or `restore` would
   destroy uncommitted edits. **Fixed**: find the marker, `git diff` each file, remove only the mutation hunk, delete only the
   untracked regression files; never `checkout`, `restore`, `reset --hard` or `clean` on files that may hold uncommitted edits.
3. LOW: the stated reason for "no retries" had the direction wrong (this was an error introduced by the lead's own correction
   note). nextest retries only after a failure, so a retried pass is green and the mutant is reported MISSED (a false MISSED), not a
   false "caught". **Fixed**.
4. LOW: `$BASE` never defined; `git diff HEAD` omits untracked new files. **Fixed**.
5. LOW: eval 5 expected something the skill never says; no eval graded the output contract, an equivalent mutant, or exit 3.
   **Fixed**.
6. INFO: the `mutants` profile inherits `[profile.default]`'s `slow-timeout` (120 s), which can fire before a larger cargo-mutants
   `--timeout` and report a hang as caught. **Documented** as `[derived]`, not run.
7. INFO: process residue in tags; stale-mtime section scoped to archive restores; three claims upgradable to `[re-run]`.
   **Fixed**.
8. INFO: `mutants = "0.0.3"` version nit, `@<ver>` in argv, seed example. **Fixed**.
9. INFO: no `allowed-tools` while the installed skills declare one; omitting it pre-approves nothing.

## Collisions
None installed. Sibling overlap was finding 1 only; mutual NOT clauses present.

## Spot-checks: 12 `[re-run]` claims, all held
18 mutants listed and 18 caught in 19 s, exit 0; either `is_empty` assertion alone gives 18 caught and both removed gives 2 MISSED,
exit 2; `--iterate` excludes 16 and catches 2; `--in-diff` lists 3 including `37:28` and catches them; exit 4 with both error
strings; the `--in-place` regressions file appears without the env var and not with it, source restored byte-identically;
`mutants.out/` layout with singular `log/`; `exclude_globs` 0 hits with the config and 3 without; `--flaky-result pass` and the
env var go green; proptest writes the file only without the env var; `.gitignore` already ignores `mutants.out*`.
Upstream confirmed by WebFetch: exit codes 0 to 6 and 70, `--in-diff` semantics, `--iterate`, the `--in-place` marker and its
interruption risk, timeout rules, `cap_lints`, `exclude_re`, `#[mutants::skip]` needing a regular dependency, nextest skipping
doctests, `cfg` not evaluated, `error_values`.

## Not verified
`--no-config` (page 404); auto-skipping of `#[cfg(test)]` and `macro_rules!` (indirect only); the `mutants.out.old` rename;
whether copy mode excludes `target/`.

## After the review
The fixes were applied after this review and were **not re-reviewed by `skill-reviewer`**. The lead re-ran the mechanical gate and
confirmed the retry-reason correction, the recovery text, the licence line and the evidence citations by reading the changed text.
