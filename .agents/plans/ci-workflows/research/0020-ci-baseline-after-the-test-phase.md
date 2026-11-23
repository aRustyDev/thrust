# Research 0020: what `ci` costs after the Test phase

The third measurement of the series ([research 0017](0017-ci-baseline.md), before the migration; [research 0019](0019-ci-baseline-after-migration.md), after it), for the
acceptance of [phase 0003](../phases/0003-test.md) (criterion 4: passes 5 to 7 are measured against 0019, not asserted), part of [ci-workflows](../PLAN.md).
Collected 2026-11-22 23:34 UTC by [`scripts/ci-baseline.py`](../scripts/ci-baseline.py) from GitHub's run and job records of `ci.yml`. The method is 0017's and 0019's and
is not repeated: runner-seconds as the sum of job executions, a run's wall clock from its final attempt, *full*, *affected-only* and *nothing* as 0017 defines them.
The script's own output is the appendix, so that every number below can be found in it.

## What was measured

**Window.** Runs created since 2026-11-19T20:43:35Z, when #341 (stage 6) merged: the last stage of the phase that changed what an existing job costs. Stages 1 to 3 (#311 to
#331) merged before it, on 2026-11-18 and 2026-11-19, stage 5 and stage 7 changed nothing, and stage 8 (#345) merged at 2026-11-22T23:31Z and added one job, `workers`, so
**only 5 of the 19 full runs have it** (its 97 s is the median of those 5; the first of them failed, on Node 22, and is in it). `ci.yml` itself did not change in the window, and
neither did the names of its checks.

26 runs fall in the window: 19 **full**, 4 **affected-only**, 2 that planned nothing but `changes` and `gate`, and 1 cancelled (a superseded pull-request run).

**The sample is a different sample from 0019's, and a good part of it is my own.** Three days, not one night. The 19 full runs are 13 pull-request runs (five of them are the
stage 8 pull request's own, #345), 3 pushes to `main` and 3 nightly runs (`schedule`, 2026-11-20 to 2026-11-22). Nobody chose them for being typical, and the p90s are wide (the
e2e cell's is 683 s over a 537 s median). Two things in it are not a steady state. **`deny` failed in 3 of 19 executions, and `gate` in 4 of 19:** the three `deny` failures are runs of
2026-11-22 between 23:01 and 23:06 UTC, on a lock with `url` 2.5.8, when the new advisory RUSTSEC-2026-0592 turned `cargo deny` red on `main`'s own lock (`main`'s push run for #343 is
one of them, so `main`'s own CI was red until #345 carried the fix). I reproduced the cause for one of the three on its tree, and did not read the logs of the other two. The fourth `gate`
failure is the first run of #345, which failed `workers` alone on Node 22. Runner contention from other sessions pushing at once is probably in these numbers too; I did not measure it.

## Before and after

Medians of the **full** runs: 0019 (2026-11-18, after the migration and before the phase; 11 runs) and this one (19 runs).

| | 0019 | now | change |
|---|---|---|---|
| jobs in a full run | 42 | 46 | +4: three more `mutants` legs, and `workers` |
| **wall clock (s), median** | **1024** | **585** | **−439 (−43%)** |
| wall clock (s), p90 | 1232 | 731 | −501 (−41%) |
| execution span (s), median | 973 | 581 | −392 (−40%) |
| **runner-seconds, median** | **5483** | **5240** | **−243 (−4%)** |
| runner-seconds, p90 | 5904 | 5470 | −434 (−7%) |
| seconds in jobs that failed, median / p90 | 0 / — | 0 / 54 | the window has two red runs |
| runs that needed a re-run | 1 of 11 | 0 of 19 | |

**The phase bought wall clock and little money.** A full run is 43% faster, and **1 to 4% cheaper, which these samples cannot tell from no change** (the two figures are below). That is
what the stages said they would do: stage 1 parallelised `ci-scripts` (more cores, the same work), stage 2 sharded `mutants` (more runners, each building, so *more* runner-seconds), and only
stages 3 and 6 removed work.

Per job (medians, seconds; the stage that touched it in brackets):

| job | 0019 | now | change | |
|---|---|---|---|---|
| `test: ci-scripts` | 305 | 122 | −183 (−60%) | stage 1: the commands run a core at a time |
| `test: mutants` | 744 (one job) | 278, 275, 272, 267 (four legs) | job −466 (−63%); **runner-seconds 744 → 1092 (+348, +47%)** | stage 2: sharded; each leg pays its own setup and build |
| `test: hermetic` | 176 | 139 | −37 (−21%) | stage 3: no debug info |
| `test: pg-adapter` | 133 | 170 | **+37 (+28%)**; p90 164 → 325 | stage 5 changed nothing; see below |
| `test: workers` | — | 97 (5 runs) | new | stage 8; 68 s once the cache is warm |
| `contract: worker-cf e2e` | 624 | 537 | −87 (−14%); p90 954 → 683 | stage 6: the browser without its apt fonts |
| `contract: worker-cf e2e-mail` | 341 | 277 | −64 (−19%) | stage 6 |
| the 26 cells, summed per run | 3568 | 3376 | −192 (−5%) | |
| the `test` jobs, summed per run | 1477 | 1597 | +120 (+8%) | the shards and `workers` cost more than `ci-scripts` and `hermetic` saved |
| `lint` (3 jobs) / `web` (2) / `security` (2) / `documentation` (2) | 135 / 67 / 34 / 20 | 143 / 71 / 34 / 19 | within the spread | not touched by this phase |

**`pg-adapter` went up, and I do not know exactly why.** Stage 5 measured it and changed nothing, and its own section says the job is bound by a container per test. What I can say
is that the job runs more tests than it did when 0019 was collected: eight pull requests touched the Postgres adapter's code or tests after 0019's window opened (#305, #308,
#315, #310, #322, #326, #332 and #336, the last of which added the purge ports), and `pg_adapter.rs` now has 129 tests. I have not counted the tests then, and I have not timed them one by one, so
none of the 37 s is attributed. The p90 doubling (164 to 325 s) says that some of those runs were slow for another reason, probably contention.

**Stage 6's own record says 768 s for the e2e cell, and this table says 624 s, because they are different windows** (stage 6 compared a 24-run window before the stage with three runs after it;
0019's window is 11 runs). Both are medians; neither is wrong. The stage's own before-and-after is in its section of the phase document.

## The critical path

[`critical-path.py`](../scripts/critical-path.py) on three full runs that succeeded (two nightlies and the last run of the stage 8 pull request; it reads only runs whose gate passed, which is why
`main`'s red push run is not here), against 0019's:

| run | wall clock (s) | the job that finished last | from its end to the gate's end |
|---|---|---|---|
| the nightly of 2026-11-22, on `749665f` | 706 | `worker-cf e2e`, 663 s | 28 s |
| the nightly of 2026-11-21, on `749665f` | 549 | `worker-cf e2e`, 508 s | 26 s |
| pull request #345 on `9432a4f` | 582 | `worker-cf e2e`, 537 s | 29 s |
| *0019: three runs after the migration* | *952 to 1229* | *`worker-cf e2e`, 916 to 1191 s* | *24 to 28 s* |

**The e2e cell sets the wall clock of every full run, as it did in 0019, and is now the only thing that does.** In the three runs 0019 read it ran 916 to 1191 s; in these three, 508 to 663 s (and in
0019 a `mutants` run of 744 s at the median could be the last job instead; no longer). The next longest job is a `mutants` leg, at about 278 s, so the e2e cell is **about 260 s ahead of everything else**, and wall clock cannot go lower without work on that cell itself, which the
plan gives to Phase 6 (E2E). What is left after the cell (the verdicts and the gate) is 26 to 29 s, as before.

## What the numbers say, and what they do not

- **Wall clock:** 43% down at the median, which is the cell and `mutants`. The two stages that did it are 2 (mutants, 744 s to 278 s a leg) and 6 (the e2e cell and its mail sibling). Stage 1's 183 s
  never showed in the wall clock of a full run, because `ci-scripts` was never the last job; it shows in a pull request that plans only it (affected-only runs: `ci-scripts` 153 s now, against 258 s in 0019).
- **Runner-seconds: 1 to 4% down, and I cannot say which.** The median of whole runs fell by 243 s (4.4%); the medians of the groups, added (contract −192, `test` +120, `lint` +8, `web` +4,
  `documentation` −1, `changes` and `gate` +3), fall by 58 s (1.1%). The two are different statistics over different samples (11 runs and 19), and I did not find what reconciles them, so I report
  both. What the groups do say: stages 1, 3 and 6 saved about 183 + 37 + 151 s, and sharding (+348 s), `workers` (+97 s in the runs that have it) and the unexplained `pg-adapter` (+37 s) spent
  more than that back in `test`. A phase of this shape is not a saving of money, and it did not claim to be one.
- **What a job costs in the share of a run:** the 26 cells are **64%** of a full run's runner-seconds (3376 of 5240 s), `test` 30%. A run gets cheaper only by running fewer cells or by making them cheaper; the
  planner already does the first for affected-only runs (4 measured: 133 s median wall clock, 233 s of runner-seconds).
- **`workers` costs 97 s cold, and 68 s with the cache** (the stage's own record has the steps): 1.9% of a run, on no critical path.

## What is in the appendix

The output of `ci-baseline.py --since 2026-11-19T20:43:35Z`, unedited. It can be run again, and stays valid until `ci.yml`'s job set or the names of its checks change.

## Appendix: the script's output

Collected 2026-11-22 23:34 UTC: 26 runs scanned, 19 full and 4 affected-only measured, 2 planned no job (only `changes` and `gate`), excluded: 1 cancelled.

### full: 19 runs

| per run | median | p90 | min | max |
|---|---|---|---|---|
| wall clock (s) | 585 | 731 | 541 | 738 |
| of which waiting before the first job (s) | 3 | 8 | 2 | 44 |
| execution span (s) | 581 | 728 | 537 | 730 |
| runner-seconds (every execution) | 5240 | 5470 | 5026 | 5539 |
| of which in jobs that failed (s) | 0 | 54 | 0 | 115 |

0 of 19 runs needed a re-run (more than one attempt).

| job | runs | median (s) | p90 (s) | failed of executions, every attempt |
|---|---|---|---|---|
| **contract, all 26 jobs (sum per run)** | 19 | 3376 | 3568 | |
| **test, all 9 jobs (sum per run)** | 19 | 1597 | 1819 | |
| contract: worker-cf e2e | 19 | 537 | 683 | 0 of 19 |
| test: mutants (0) | 19 | 278 | 329 | 0 of 19 |
| contract: worker-cf e2e-mail | 19 | 277 | 325 | 0 of 19 |
| test: mutants (2) | 19 | 275 | 321 | 0 of 19 |
| test: mutants (1) | 19 | 272 | 371 | 0 of 19 |
| test: mutants (3) | 19 | 267 | 305 | 0 of 19 |
| contract: worker-cf oracle | 19 | 195 | 222 | 0 of 19 |
| test: pg-adapter | 19 | 170 | 325 | 0 of 19 |
| contract: worker-cf main | 19 | 161 | 180 | 0 of 19 |
| contract: worker-cf sweeper | 19 | 146 | 157 | 0 of 19 |
| **lint, all 3 jobs (sum per run)** | 19 | 143 | 156 | |
| test: hermetic | 19 | 139 | 193 | 0 of 19 |
| contract: native-pg oracle | 19 | 137 | 146 | 0 of 19 |
| contract: worker-cf worker | 19 | 133 | 144 | 0 of 19 |
| contract: worker-cf cron | 19 | 127 | 135 | 0 of 19 |
| test: ci-scripts | 19 | 122 | 182 | 0 of 19 |
| contract: worker-cf panic | 19 | 122 | 137 | 0 of 19 |
| contract: worker-cf bindings | 19 | 121 | 135 | 0 of 19 |
| contract: worker-cf mail | 19 | 119 | 134 | 0 of 19 |
| contract: worker-memory main | 19 | 118 | 124 | 0 of 19 |
| contract: worker-memory sweeper | 19 | 110 | 121 | 0 of 19 |
| contract: native-memory oracle | 19 | 108 | 116 | 0 of 19 |
| contract: worker-memory cron | 19 | 108 | 112 | 0 of 19 |
| contract: worker-memory csrfoff | 19 | 104 | 111 | 0 of 19 |
| contract: worker-memory panic | 19 | 99 | 113 | 0 of 19 |
| contract: worker-memory mail | 19 | 99 | 111 | 0 of 19 |
| test: workers | 5 | 97 | 106 | 1 of 5 |
| contract: native-pg main | 19 | 83 | 95 | 0 of 19 |
| contract: native-pg sweeper | 19 | 80 | 92 | 0 of 19 |
| contract: native-pg panic | 19 | 76 | 84 | 0 of 19 |
| **web, all 2 jobs (sum per run)** | 19 | 71 | 91 | |
| lint: deps | 19 | 70 | 76 | 0 of 19 |
| lint: lint | 19 | 63 | 74 | 0 of 19 |
| contract: native-memory mail | 19 | 62 | 74 | 0 of 19 |
| contract: native-memory sweeper | 19 | 62 | 69 | 0 of 19 |
| web: web | 19 | 60 | 81 | 0 of 19 |
| contract: native-memory main | 19 | 59 | 69 | 0 of 19 |
| contract: native-memory csrfoff | 19 | 57 | 61 | 0 of 19 |
| contract: native-memory panic | 19 | 54 | 59 | 0 of 19 |
| **security, all 2 jobs (sum per run)** | 19 | 34 | 45 | |
| security: deny | 19 | 25 | 34 | 3 of 19 |
| **documentation, all 2 jobs (sum per run)** | 19 | 19 | 29 | |
| changes | 19 | 12 | 14 | 0 of 19 |
| lint: verdict | 19 | 11 | 15 | 0 of 19 |
| test: verdict | 19 | 11 | 14 | 1 of 19 |
| documentation: docs | 19 | 10 | 15 | 0 of 19 |
| web: verdict | 19 | 10 | 15 | 0 of 19 |
| gate | 19 | 10 | 13 | 4 of 19 |
| documentation: verdict | 19 | 9 | 14 | 0 of 19 |
| security: verdict | 19 | 7 | 14 | 3 of 19 |

### affected: 4 runs

| per run | median | p90 | min | max |
|---|---|---|---|---|
| wall clock (s) | 133 | 904 | 53 | 904 |
| of which waiting before the first job (s) | 4 | 709 | 3 | 709 |
| execution span (s) | 123 | 208 | 49 | 208 |
| runner-seconds (every execution) | 233 | 426 | 41 | 426 |
| of which in jobs that failed (s) | 0 | 0 | 0 | 0 |

0 of 4 runs needed a re-run (more than one attempt).

| job | runs | median (s) | p90 (s) | failed of executions, every attempt |
|---|---|---|---|---|
| **test, all 3 jobs (sum per run)** | 2 | 328 | 330 | |
| test: pg-adapter | 2 | 163 | 165 | 0 of 2 |
| test: ci-scripts | 2 | 153 | 157 | 0 of 2 |
| **lint, all 2 jobs (sum per run)** | 2 | 77 | 82 | |
| lint: deps | 2 | 68 | 72 | 0 of 2 |
| **documentation, all 2 jobs (sum per run)** | 2 | 20 | 23 | |
| changes | 4 | 14 | 15 | 0 of 4 |
| documentation: docs | 2 | 12 | 13 | 0 of 2 |
| test: verdict | 2 | 12 | 15 | 0 of 2 |
| gate | 4 | 9 | 11 | 0 of 4 |
| lint: verdict | 2 | 8 | 10 | 0 of 2 |
| documentation: verdict | 2 | 8 | 10 | 0 of 2 |

### nothing: 2 runs

wall clock median 30 s, p90 34 s; of which waiting before the first job median 3 s, p90 3 s.

### runs measured

- full: 45082538802 (pull_request), 45081430725 (pull_request), 45081065114 (pull_request), 45080900139 (push), 44980810575 (schedule), 44861340508 (schedule), 44827044680 (pull_request), 44826225211 (pull_request), 44825424412 (pull_request), 44822700250 (pull_request), 44821018977 (pull_request), 44818988484 (pull_request), 44812894172 (pull_request), 44812679808 (push), 44668102080 (schedule), 44638148725 (pull_request), 44631225818 (pull_request), 44611805686 (pull_request), 44611594659 (push)
- affected: 45080939758 (push), 44826984973 (pull_request), 44820832071 (push), 44820420971 (pull_request)

