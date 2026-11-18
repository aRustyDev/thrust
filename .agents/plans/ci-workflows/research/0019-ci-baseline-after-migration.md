# Research 0019: what `ci` costs after the migration

The second measurement that [research 0017](0017-ci-baseline.md) promised ("the same measurement is taken again after the migration"), for the
migration's acceptance ([phase 0002](../phases/0002-migration.md)), part of [ci-workflows](../PLAN.md). Collected 2026-11-18 04:19 UTC by
[`scripts/ci-baseline.py`](../scripts/ci-baseline.py) from GitHub's run and job records of `ci.yml`. It is what passes 5 to 7 are measured
against now that the layout is final. The method is 0017's and is not repeated: runner-seconds as the sum of job executions, a run's wall clock
from the final attempt, *full*, *affected-only* and *nothing* as 0017 defines them.

## What was measured

**Window.** Runs created since 2026-11-17T21:42:42Z, when #281 (M5b) merged. That was the last change to `ci.yml`'s job set and to the names of
its checks: after it `ci.yml` holds `changes`, five calls and `gate`, and a contract cell's check is `test / contract (<target>, <pass>) /
<target> <pass>`. (M6 renamed a workflow `ci` does not call; the pull requests since changed steps and the gate's rules, not the set of jobs.)
22 runs fall in it: 4 were cancelled (superseded), 11 were **full** and 7 **affected-only**, and none planned only `changes` and `gate`.

**The sample is small, and it is not 0017's kind of sample.** About six and a half hours of one busy night. The 11 full runs are: the nightly (`schedule`),
a `workflow_dispatch`, three `rc/*` pull-request runs, three other pull requests, and three pushes to `main`. The 7 affected-only runs
are six pull requests and one push, and **two of the seven are my own deliberate probes (#291 and #292)**, which failed on purpose: the "failed" rows of
the affected table are theirs. Several sessions were pushing at once, so runner contention is probably in these numbers (the p90s are wide); I did not measure it.
Run the script again later with the same `--since`; it stays valid until `ci.yml`'s job set changes.

**The script needed one change for the new layout, and had it already.** Cells are classified by their own shape (`CELL` in `ci-baseline.py`, #281), so
that a cell stays of the group `contract` whether or not a caller precedes it. On the real run records it grouped 26 cells under `contract` and the five jobs of
`test` under `test`.

## What the numbers say, and what they do not

Medians of the full runs, against 0017's 18 (2026-11-16, before the migration):

| | before (0017, 18 runs) | after (this, 11 runs) |
|---|---|---|
| jobs in a full run | 37 (the push run on `b83ece3`, the last commit before M1) | 42 |
| wall clock (s), median / p90 | 766 / 1136 | 1024 / 1232 |
| execution span (s), median | 750 | 973 |
| runner-seconds, median / p90 | 4552 / 5269 | 5483 / 5904 |
| seconds in jobs that failed, median | 74 | 0 |
| runs that needed a re-run | 4 of 18 | 1 of 11 |

A full run costs **931 runner-seconds more (20%)** and takes **258 s longer (34%)** at the median. **Most of that is not the migration's**, and the data
separate what is:

- **The migration's own cost is five `verdict` jobs**: medians of 8 (`documentation`), 11 (`security`), 7 (`web`), 7 (`lint`) and 10 s (`test`), **43 runner-seconds,
  0.8% of a full run**. They are the whole difference in the count of jobs (37 + 5 = 42): [`compare-job-sets.py`](../scripts/compare-job-sets.py) strips the workflow prefix the migration adds to
  a job's name and finds nothing in the run before that is missing from the run after, the same 26 cells included, and `verdict` as the only name added.
- **`ci-scripts` went from 71 s to 305 s (+234 s)**: it now runs the mutant driver and the wiring test (Phase 0 and M1 put them there, after 0017's window) and the
  tests that other pull requests added (the deploy target, the preview control plane). The same commands, in a bigger list.
- **The contract cells went from 3198 s to 3568 s a run (+370 s)**: a 26th cell, `worker-cf e2e-mail` (341 s at the median), which was in the matrix at `b83ece3`
  and was not in 0017's window (25 cells). Not the migration's either.
- `mutants` (692 to 744 s), `hermetic` (162 to 176 s) and `pg-adapter` (123 to 133 s) run the same commands; the differences are within the spread (the `mutants`
  p90 went from 868 to 1019 s).

**Wall clock.** I read the critical path of one full run before and three after, with [`critical-path.py`](../scripts/critical-path.py):

| run | wall clock (s) | the job that finished last | from its end to the gate's end |
|---|---|---|---|
| push on `b83ece3`, before | 858 | `mutants`, 838 s | the gate started 3 s later and ran 6 s: **9 s** |
| dispatch on `564c929` | 973 | `worker-cf e2e`, 935 s | the last `verdict` ended 10 s later, the gate started 4 s after that and ran 11 s: **25 s** |
| the nightly on `564c929` | 1229 | `worker-cf e2e`, 1191 s | 13 s, 2 s, 9 s: **24 s** |
| an `rc/*` run on `caa13b3` | 952 | `worker-cf e2e`, 916 s | 18 s, 3 s, 7 s: **28 s** |

So **the migration adds 15 to 19 s to the critical path of a run (1.2 to 2%)**: the last `verdict` job, which has to run after the last job, and the gate after it. The rest
of the difference is the longest job: in these three runs `worker-cf e2e` took 916 to 1191 s, against a median of 624 s over the 11 (p90 954 s). I did **not**
isolate why (runner contention is the first suspect, a cold cache on the nightly another); it is not the migration's to explain, and it is where pass 6
(parallelization) and the E2E phase start.

**Affected-only runs** (7, two of them probes): wall clock median 314 s, p90 416 s; runner-seconds median 406 s, against 0017's 5 runs. Too few to compare, as in 0017.
A pull request that edits a workflow file, the book and the plans (#289) planned `documentation`, `lint` (`deps` only) and `test` (`ci-scripts` only) and ran 8 of 15 jobs.

## To repeat it

```sh
uv run --script .agents/plans/ci-workflows/scripts/ci-baseline.py --since 2026-11-17T21:42:42Z     # the default --groups follow the migration
uv run --script .agents/plans/ci-workflows/scripts/ci-baseline.py --self-test                      # 44 checks, no network
python3 -I .agents/plans/ci-workflows/scripts/compare-job-sets.py aRustyDev/thrust 44008843071 44216506614   # the job sets, before and after; exit 1 if one is lost
python3 -I .agents/plans/ci-workflows/scripts/critical-path.py aRustyDev/thrust 44008843071 44216506614 44230791965 44214903508
```

The first comparison has its control: given the affected-only run `44187850687` as the run after, it reports 27 jobs lost (all 26 cells and `deny`) and exits 1.

## The script's output, unedited

Collected 2026-11-18 04:19 UTC: 22 runs scanned, 11 full and 7 affected-only measured, 0 planned no job (only `changes` and `gate`), excluded: 4 cancelled.

### full: 11 runs

| per run | median | p90 | min | max |
|---|---|---|---|---|
| wall clock (s) | 1024 | 1232 | 112 | 1488 |
| of which waiting before the first job (s) | 3 | 258 | 2 | 301 |
| execution span (s) | 973 | 1187 | 107 | 1229 |
| runner-seconds (every execution) | 5483 | 5904 | 5215 | 6052 |
| of which in jobs that failed (s) | 0 | 0 | 0 | 95 |

1 of 11 runs needed a re-run (more than one attempt).

| job | runs | median (s) | p90 (s) | failed of executions, every attempt |
|---|---|---|---|---|
| **contract, all 26 jobs (sum per run)** | 11 | 3568 | 4342 | |
| **test, all 5 jobs (sum per run)** | 11 | 1477 | 1594 | |
| test: mutants | 11 | 744 | 1019 | 0 of 11 |
| contract: worker-cf e2e | 11 | 624 | 954 | 0 of 11 |
| contract: worker-cf e2e-mail | 11 | 341 | 515 | 0 of 11 |
| test: ci-scripts | 11 | 305 | 375 | 0 of 11 |
| contract: worker-cf oracle | 11 | 207 | 238 | 0 of 11 |
| test: hermetic | 11 | 176 | 202 | 0 of 11 |
| contract: worker-cf main | 11 | 174 | 196 | 0 of 11 |
| contract: worker-cf sweeper | 11 | 147 | 164 | 0 of 11 |
| contract: native-pg oracle | 11 | 145 | 148 | 0 of 11 |
| contract: worker-cf worker | 11 | 140 | 149 | 0 of 11 |
| **lint, all 3 jobs (sum per run)** | 11 | 135 | 144 | |
| test: pg-adapter | 11 | 133 | 164 | 0 of 11 |
| contract: worker-cf cron | 11 | 131 | 138 | 0 of 11 |
| contract: worker-cf panic | 11 | 125 | 144 | 0 of 11 |
| contract: worker-cf bindings | 11 | 122 | 128 | 0 of 11 |
| contract: worker-cf mail | 11 | 121 | 127 | 0 of 11 |
| contract: worker-memory main | 11 | 109 | 123 | 0 of 11 |
| contract: native-memory oracle | 11 | 108 | 114 | 0 of 11 |
| contract: worker-memory sweeper | 11 | 107 | 114 | 0 of 11 |
| contract: worker-memory panic | 11 | 104 | 111 | 0 of 11 |
| contract: worker-memory mail | 11 | 102 | 110 | 0 of 11 |
| contract: worker-memory cron | 11 | 101 | 113 | 0 of 11 |
| contract: worker-memory csrfoff | 11 | 101 | 124 | 0 of 11 |
| contract: native-pg main | 11 | 93 | 95 | 1 of 12 |
| contract: native-pg sweeper | 11 | 90 | 95 | 0 of 11 |
| contract: native-pg panic | 11 | 87 | 89 | 0 of 11 |
| lint: deps | 11 | 71 | 79 | 0 of 11 |
| **web, all 2 jobs (sum per run)** | 11 | 67 | 87 | |
| contract: native-memory main | 11 | 64 | 68 | 0 of 11 |
| contract: native-memory mail | 11 | 63 | 72 | 0 of 11 |
| web: web | 11 | 60 | 79 | 0 of 11 |
| lint: lint | 11 | 58 | 61 | 0 of 11 |
| contract: native-memory sweeper | 11 | 54 | 63 | 0 of 11 |
| contract: native-memory panic | 11 | 53 | 59 | 0 of 11 |
| contract: native-memory csrfoff | 11 | 49 | 57 | 0 of 11 |
| **security, all 2 jobs (sum per run)** | 11 | 34 | 36 | |
| security: deny | 11 | 23 | 26 | 0 of 11 |
| **documentation, all 2 jobs (sum per run)** | 11 | 20 | 29 | |
| documentation: docs | 11 | 14 | 17 | 0 of 11 |
| security: verdict | 11 | 11 | 13 | 0 of 11 |
| changes | 11 | 10 | 15 | 0 of 11 |
| test: verdict | 11 | 10 | 15 | 1 of 12 |
| gate | 11 | 9 | 11 | 1 of 12 |
| documentation: verdict | 11 | 8 | 13 | 0 of 11 |
| web: verdict | 11 | 7 | 13 | 0 of 11 |
| lint: verdict | 11 | 7 | 11 | 0 of 11 |

### affected: 7 runs

| per run | median | p90 | min | max |
|---|---|---|---|---|
| wall clock (s) | 314 | 416 | 152 | 416 |
| of which waiting before the first job (s) | 4 | 38 | 2 | 38 |
| execution span (s) | 311 | 412 | 144 | 412 |
| runner-seconds (every execution) | 406 | 641 | 222 | 641 |
| of which in jobs that failed (s) | 0 | 239 | 0 | 239 |

0 of 7 runs needed a re-run (more than one attempt).

| job | runs | median (s) | p90 (s) | failed of executions, every attempt |
|---|---|---|---|---|
| **test, all 4 jobs (sum per run)** | 7 | 267 | 538 | |
| test: ci-scripts | 7 | 258 | 376 | 2 of 7 |
| test: hermetic | 1 | 215 | 215 | 0 of 1 |
| test: pg-adapter | 1 | 154 | 154 | 0 of 1 |
| **lint, all 3 jobs (sum per run)** | 6 | 74 | 121 | |
| lint: deps | 6 | 67 | 78 | 1 of 6 |
| lint: lint | 1 | 41 | 41 | 0 of 1 |
| **documentation, all 2 jobs (sum per run)** | 6 | 23 | 28 | |
| documentation: docs | 6 | 12 | 16 | 0 of 6 |
| lint: verdict | 6 | 9 | 12 | 1 of 6 |
| documentation: verdict | 6 | 8 | 17 | 0 of 6 |
| changes | 7 | 8 | 14 | 0 of 7 |
| test: verdict | 7 | 8 | 14 | 2 of 7 |
| gate | 7 | 8 | 11 | 2 of 7 |

### runs measured

- full: 44230791965 (schedule), 44216506614 (workflow_dispatch), 44214903508 (pull_request), 44214380128 (pull_request), 44213845332 (push), 44189032295 (pull_request), 44188800112 (push), 44187852413 (pull_request), 44185674792 (pull_request), 44182914506 (pull_request), 44182653438 (push)
- affected: 44214410113 (pull_request), 44214345793 (pull_request), 44213806914 (push), 44213278632 (pull_request), 44187984775 (pull_request), 44187918998 (pull_request), 44187850687 (pull_request)
