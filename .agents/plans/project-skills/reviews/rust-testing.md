# Review: rust-testing (round 1)

Reviewer: skill-reviewer. Draft read in full (SKILL.md, 8 references, 2 eval files). Spike HEAD at review: 620e2a1.

**Verdict: REJECT. Grade: UNGRADEABLE (Section A failed).**

## Section A (blocking)
- A2.1 overly broad tool permissions: `Bash(scripts/rs:*)` (SKILL.md:21). Same trampoline as rust-on-workers; used only once, in `references/must-fail-controls.md:76`. Fix: delete, or `Bash(scripts/rs cargo tree:*)`.
- The other five `Bash(scripts/...:*)` entries are fixed-argument scripts: fine. All other Section A checks passed.

## Section B/C (to fix in the same pass)
2. Machine/process-specific text in shipped files (authoring-machine parentheticals, spend-limit narrative, "plan row", "PR-2").
3. Hand-maintained numbers restated outside the scripts (cell counts, EXPECT_* values, testcontainers versions, kill counts): point at the source instead.
4. SOURCES.md status tables go stale by construction; install-time mechanical path check recommended (extract backticked repo paths, `git ls-files` on merged main, assert guard strings still present).
5. SKILL.md body duplicates references (procedure appears 3x); trim to ~2.5-3 KB.
6. Time-bound wording in references; correct `contract.rs:5` header drift upstream instead.
7. Four NOT-YET-USED references: keep in this skill (splitting would collide on trigger); add promotion/removal rule + last-reviewed stamps.
8. Shared legend for `[derived]`/`[spike]`/`[re-run]`.
9-14. Misquote of mutants.sh:5; overstated admin-token claim; untagged 1-in-4 claim; eval 3 printed line wrong; trigger phrase needs a Rust/thrust anchor; imprecise mutant wiring wording.
15. Add a worked sample of summary.txt; avoid secret-scanner-bait URL; note container image pulls.

## Judgments on author-flagged items
- Description length (756 chars): acceptable. `allowed-tools` `:*` syntax: valid; defect is the scripts/rs entry. 4,041 B: criterion is proportionality, trim duplication. References 45 KB: unread references cost zero tokens; real cost is staleness of unexercised digests. `[derived]`: acceptable with a shared legend.

## Spot-checks: all held against the spike
mutants.sh kill rule (exit 101 + FAILED line; build failure is not a kill), mutant wiring, flake-hunt.sh exits 0, memory-vs-postgres rollback difference (quoted exactly), contract.sh exit codes and EXPECT_* values, contract-all.sh 16 cells, check-deps.sh, blast/redelivery tests, testcontainers versions. Not re-run: kill counts, mistyped `-p` behaviour, nextest/mutants commands.

## Collisions: none blocking (rust-on-workers adjacent by design; terraform-test, provider-test-patterns, run-acceptance-tests, dioxus-07 excluded explicitly).
