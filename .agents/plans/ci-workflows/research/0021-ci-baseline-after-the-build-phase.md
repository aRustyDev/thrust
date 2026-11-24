# Research 0021: what `ci` costs after the Build phase

The fourth measurement of the series ([research 0017](0017-ci-baseline.md), before the migration; [0019](0019-ci-baseline-after-migration.md), after it; [0020](0020-ci-baseline-after-the-test-phase.md),
after the Test phase), for the acceptance of [phase 0004](../phases/0004-build.md) (criterion 4: passes 5 to 7 are measured against 0020, not asserted), part of [ci-workflows](../PLAN.md).
Collected 2026-11-24 04:50 UTC by [`scripts/ci-baseline.py`](../scripts/ci-baseline.py) from GitHub's run and job records of `ci.yml`, and the same day from the records of `build-extended.yml`. The method is
0017's and 0019's and is not repeated: runner-seconds as the sum of job executions, a run's wall clock from its final attempt, *full*, *affected-only* and *nothing* as 0017 defines them. The script's own
output is the appendix, so that every number below can be found in it.

## What was measured

**Window.** Runs created since 2026-11-23T06:40:43Z, when #367 (stage 4) merged: the last change to a job of the gate that Build added. Stages 1 and 2 merged earlier that day (#352 at 01:39Z, #356 at 03:47Z), and
#367 changed how `linux` saves its cache. **Stages 3 to 6 are not in `ci`**: `build-extended.yml` is not called by `ci.yml` and `gate` does not read it, so they cost `ci` nothing, and their cost is measured
separately below. The job set of a full run changed in the window in one more way that is not Build's: the Lint phase's `bash`, `yaml` and `json` jobs (the last only in the 6 of 18 runs after it merged).

48 runs fall in the window: 18 **full**, 26 **affected-only**, none that planned nothing, and 4 cancelled. **The 18 full runs are 13 pull-request runs, 4 pushes to `main` and 1 nightly**
(`schedule`, 2026-11-24). Nobody chose them for being typical: a good part of the pull-request runs are other sessions' own, which matters for the wall clock (below). One run needed a re-run.

## Before and after

Medians of the **full** runs: 0020 (2026-11-22; 19 runs) and this one (18 runs).

| | 0020 | now | change |
|---|---|---|---|
| jobs in a full run | 46 | 52 | +6: `build` has three (`linux`, `wasm`, `verdict`), and Lint's `bash`, `yaml` and `json` are three more |
| **wall clock (s), median** | **585** | **652** | **+67 (+11%)** (see *The wall clock*: not Build's) |
| wall clock (s), p90 | 731 | 793 | +62 |
| execution span (s), median | 581 | 617 | +36 |
| **runner-seconds, median** | **5240** | **5607** | **+367 (+7%)** |
| runner-seconds, p90 | 5470 | 5861 | +391 |
| seconds in jobs that failed, median / p90 | 0 / 54 | 0 / 122 | one run needed a re-run |
| runs that needed a re-run | 0 of 19 | 1 of 18 | |

**The phase added a gate job family and no money worth the name: `build` is 196 s of a 5607 s run (3.5%), on no critical path.** What it cost, per job (medians, seconds):

| group | 0020 | now | change | what moved it |
|---|---|---|---|---|
| **`build`** (3 jobs) | — | **196** | **+196** | new: `linux` 104 (warm: its cache is saved on `main`), `wasm` 86, `verdict` 10 |
| `lint` | 143 (3 jobs) | 200 (6 jobs) | +57 | **not Build's**: `bash` 39, `yaml` 11, `json` 12 (6 runs) are the Lint phase's new jobs |
| `test` (sum per run) | 1597 | 1678 | +81 | `ci-scripts` 122 → 154 (+32), `workers` 97 → 68 (−29: 0020's median is of 5 cold runs), `hermetic` 139 → 131, `pg-adapter` 170 → 163, the four `mutants` legs 278, 275, 272, 267 → 266, 272, 255, 292 |
| the 26 cells (sum per run) | 3376 | 3364 | −12 | within the spread; `worker-cf e2e` 537 → 532, `e2e-mail` 277 → 276 |
| `web` / `security` / `documentation` | 71 / 34 / 19 | 68 / 32 / 20 | within the spread | not touched |

**`ci-scripts` went up by 32 s while its mutant driver went from 272 to 515 mutants.** That is a lead, not an attribution: I did not time the driver's share of the job. The driver is one of its commands
(`ci-scripts` runs the commands a core at a time, [stage 1 of phase 0003](../phases/0003-test.md)), and the stage records give Build's share of the 243 new mutants as at least 162 (stage 1 added 41, stage 2 28,
stage 5 65, stage 6 28; stages 3 and 4 added some more that I did not count); the Lint phase's stages added the rest.

**The groups account for +318 of the +367 s, and about 49 s are not found.** `build` +196, `lint` +57 and `test` +81 are +334; the cells, `web`, `security` and `documentation` are −16 (−12, −3, −2, +1); `changes` and `gate`
are within a second or two. Medians of whole runs and medians of groups are different statistics (0020 had the same gap, and said so), so I report both and do not reconcile them.

**Affected-only runs** (a pull request or a push that plans some jobs): 26 now (median wall clock 222 s, runner-seconds 377), against 4 in 0020 (133 s and 233 s). They are different samples: the 4 in 0020 were small,
and most of today's 26 are pull requests that change workflows and scripts, which plan `ci-scripts` (154 s), `deps` (84 s) and `yaml`. I did not compare like with like, so I do not read the difference as a cost.

## The wall clock

**The median went from 585 to 652 s, and a clean run did not.** `critical-path.py` on four full runs that passed (the nightly and three pushes to `main`), against 0020's:

| run | wall clock (s) | the job that finished last | from its end to the gate's end |
|---|---|---|---|
| the nightly of 2026-11-24, on `485b284` | 566 | `worker-cf e2e`, 531 s | 23 s |
| push, `4ed6d0b` | 568 | `worker-cf e2e`, 532 s | 25 s |
| push, `c5ad8f0` (the commit of #367) | 598 | `worker-cf e2e`, 514 s (it started 67 s in) | 17 s |
| push, `4ca981e` | 567 | `worker-cf e2e`, 528 s | 21 s |
| *0020: three runs* | *549 to 706* | *`worker-cf e2e`, 508 to 663 s* | *26 to 29 s* |

**The e2e cell still sets the wall clock of every full run, and Build is not on the critical path**: `build / linux` (104 s) and `build / wasm` (86 s) start with the other jobs and are done minutes before the e2e cell ends.
**What lifts the median is the pull-request runs** ([`full-runs.py`](../scripts/full-runs.py), in the appendix). The 12 complete ones span 596 to 814 s (median 674 s); the 5 clean runs (the nightly and four pushes) span 568 to 601 s. **The cell itself ran longer in them**: 498 to 750 s
(median 547 s; four of the twelve took 638 to 750 s), against 514 to 532 s in the clean runs. The longest wait for a runner in a run (from the end of `changes` to the start of a job; verdicts and the gate are left out, because
they wait for their jobs, not for a runner) was 27 to 162 s in the pull-request runs (median 74 s) and 28 to 120 s in the clean ones (median 54 s): a modest difference with a large overlap, and in every run the longest wait is a
contract cell. A slower cell and a longer queue are what other pull requests running at once would do to a repository's runners, and a sample of pull-request runs picks that up and a nightly does not; **I did not
measure that cause**, I measured that the cell is slower in those runs and that Build's jobs are not the last ones.
**One of the 18 "full" runs is a partial re-run of 141 s** (attempt 2: only the failed job ran again), which pulls the median down, not up.

**I cannot separate the contention from Build's three extra jobs.** A full run starts 52 jobs where it started 46, and the number of runners at once is limited; the longest waits above are of contract cells, not of `build`. 0020 did not record queue waits, so there is nothing to compare them with. What I can say is that the clean runs did not get longer.

## The extended jobs

The four jobs of `build-extended.yml` are not in `ci` and are not in the table above. Measured from the API by [`extended-times.py`](../scripts/extended-times.py), over **57 runs of `build-extended.yml`** (2 left out: the throwaway probe branches), every execution that succeeded:

| job | runs | min (s) | median (s) | max (s) | stage |
|---|---|---|---|---|---|
| `macos` | 44 | 114 | 167 | 249 | 3 |
| `windows` | 35 | 133 | 208 | 310 | 4 |
| `docker` | 11 | 188 | 219 | 329 | 5 |
| `bsd` | 4 | 184 | 214 | 218 | 6 |

`bsd` is the job as merged; its two proof-of-concept forms (`bsd-cpa` 236 s once, `bsd-vm` 264 and 450 s) are left out of the median. **No execution of these jobs on a real branch failed**, but one: the first run of the
proof of concept's `bsd-cpa` failed in 31 s at `pkg` (stage 6, finding 1). (Executions cancelled because a newer push replaced the run are in neither table; the two probes, one of which planted a break on purpose, are
left out.) No job is on a critical path or in the gate. They run when a release candidate is pushed (after every merge), nightly and by hand: a run of
all four is 167 + 208 + 219 + 214 = 808 s of runner time at the medians, on runners that are free in a public repository (macOS and Windows: the stage records say so; I did not re-read GitHub's pricing for this document).

## What the numbers say, and what they do not

- **Build's price is 196 s a full run and nothing on the clock.** The gated half of the phase (`linux`, `wasm`) is 3.5% of a full run's runner-seconds, runs beside the 26 cells and finishes 400 s before the e2e cell. The
  four extended jobs cost `ci` nothing.
- **Wall clock: unchanged in a clean run** (566 to 598 s against 0020's 549 to 706 s). The median over 18 runs is higher because of the pull-request runs, and I attribute that to contention without having measured it
  against 0020's, which did not record it.
- **Runner-seconds: +7%, of which Build is a bit more than half** (196 of 367 s); the Lint phase's new jobs are most of the rest. A phase that adds proofs adds work; this one said it would, and the stage
  sections say what each cost.
- **Passes 5 to 7 were measured stage by stage and are recorded there**: stage 1's cache (134 s → 101 s), stage 2's (238 s → 79 s, and the bundle restores it: a release candidate's build 310 s → 165 s), stages 3 and 4's
  (the flat layout, 249 s → 134 s and 219 s → 133 s). **Stages 5 and 6 added no cache, with the numbers** (`docker` writes none, `bsd` writes none, and the one candidate that wrote 1.0 GB per ref was not chosen).
- **Not measured:** the contention (0020 has no queue waits); the share of `ci-scripts`'s +32 s that the mutant driver is; the 33 s that the groups do not explain; the first night with every job on its own saved cache
  (the night of 2026-11-24, 06:11 UTC, had not run when this was collected, and is not read here).

## What is in the appendix

Three outputs, unedited. **A:** `ci-baseline.py --since 2026-11-23T06:40:43Z --groups "test lint contract web documentation security build"` (`build` is not in the script's default groups: `--groups` has to name it,
and the script refuses a job group it does not know). **B:** `full-runs.py` on A, the 18 full runs one by one; its wall clock is the run's `updated_at` less its `run_started_at`, so its median (662 s) is 10 s from A's
(652 s, to the last job's completion): the same 18 runs. **C:** `extended-times.py`. A and B can be run again and stay valid until `ci.yml`'s job set or the names of its checks change; C grows with every run.

## Appendix A: the output of `ci-baseline.py`


Collected 2026-11-24 04:50 UTC: 48 runs scanned, 18 full and 26 affected-only measured, 0 planned no job (only `changes` and `gate`), excluded: 4 cancelled.

### full: 18 runs

| per run | median | p90 | min | max |
|---|---|---|---|---|
| wall clock (s) | 652 | 793 | 140 | 813 |
| of which waiting before the first job (s) | 3 | 54 | 2 | 75 |
| execution span (s) | 617 | 759 | 135 | 790 |
| runner-seconds (every execution) | 5607 | 5861 | 5349 | 5999 |
| of which in jobs that failed (s) | 0 | 122 | 0 | 134 |

1 of 18 runs needed a re-run (more than one attempt).

| job | runs | median (s) | p90 (s) | failed of executions, every attempt |
|---|---|---|---|---|
| **contract, all 26 jobs (sum per run)** | 18 | 3364 | 3605 | |
| **test, all 9 jobs (sum per run)** | 18 | 1678 | 1776 | |
| contract: worker-cf e2e | 18 | 532 | 712 | 0 of 18 |
| test: mutants (1) | 18 | 292 | 351 | 0 of 18 |
| contract: worker-cf e2e-mail | 18 | 276 | 338 | 0 of 18 |
| test: mutants (2) | 18 | 272 | 339 | 0 of 18 |
| test: mutants (0) | 18 | 266 | 329 | 0 of 18 |
| test: mutants (3) | 18 | 255 | 293 | 0 of 18 |
| contract: worker-cf oracle | 18 | 202 | 237 | 0 of 18 |
| **lint, all 6 jobs (sum per run)** | 18 | 200 | 234 | |
| **build, all 3 jobs (sum per run)** | 18 | 196 | 212 | |
| contract: worker-cf main | 18 | 164 | 189 | 0 of 18 |
| test: pg-adapter | 18 | 163 | 187 | 0 of 18 |
| test: ci-scripts | 18 | 154 | 234 | 0 of 18 |
| contract: worker-cf sweeper | 18 | 144 | 163 | 0 of 18 |
| test: hermetic | 18 | 131 | 181 | 1 of 18 |
| contract: native-pg oracle | 18 | 130 | 138 | 0 of 18 |
| contract: worker-cf worker | 18 | 130 | 137 | 0 of 18 |
| contract: worker-cf cron | 18 | 120 | 131 | 1 of 18 |
| contract: worker-memory main | 18 | 120 | 126 | 0 of 18 |
| contract: worker-cf bindings | 18 | 118 | 126 | 0 of 18 |
| contract: worker-cf panic | 18 | 116 | 129 | 0 of 18 |
| contract: worker-cf mail | 18 | 116 | 137 | 0 of 18 |
| contract: worker-memory sweeper | 18 | 112 | 122 | 0 of 18 |
| contract: native-memory oracle | 18 | 109 | 114 | 0 of 18 |
| contract: worker-memory cron | 18 | 106 | 113 | 0 of 18 |
| build: linux | 18 | 104 | 107 | 0 of 18 |
| contract: worker-memory csrfoff | 18 | 102 | 109 | 1 of 19 |
| contract: worker-memory mail | 18 | 102 | 112 | 0 of 18 |
| contract: worker-memory panic | 18 | 100 | 111 | 0 of 18 |
| build: wasm | 18 | 86 | 94 | 0 of 18 |
| contract: native-pg main | 18 | 86 | 95 | 0 of 18 |
| lint: deps | 18 | 84 | 103 | 0 of 18 |
| contract: native-pg sweeper | 18 | 80 | 85 | 0 of 18 |
| contract: native-pg panic | 18 | 72 | 83 | 0 of 18 |
| test: workers | 18 | 68 | 76 | 0 of 18 |
| **web, all 2 jobs (sum per run)** | 18 | 68 | 88 | |
| contract: native-memory mail | 18 | 63 | 70 | 0 of 18 |
| lint: lint | 18 | 62 | 65 | 0 of 18 |
| contract: native-memory main | 18 | 60 | 67 | 0 of 18 |
| web: web | 18 | 59 | 78 | 0 of 18 |
| contract: native-memory panic | 18 | 56 | 63 | 0 of 18 |
| contract: native-memory csrfoff | 18 | 54 | 60 | 0 of 18 |
| contract: native-memory sweeper | 18 | 54 | 67 | 0 of 18 |
| lint: bash | 18 | 39 | 44 | 0 of 18 |
| **security, all 2 jobs (sum per run)** | 18 | 32 | 39 | |
| security: deny | 18 | 26 | 33 | 0 of 18 |
| **documentation, all 2 jobs (sum per run)** | 18 | 20 | 25 | |
| lint: json | 6 | 12 | 45 | 0 of 6 |
| test: verdict | 18 | 12 | 15 | 3 of 19 |
| documentation: docs | 18 | 11 | 16 | 0 of 18 |
| lint: yaml | 13 | 11 | 17 | 0 of 13 |
| web: verdict | 18 | 10 | 14 | 0 of 18 |
| build: verdict | 18 | 10 | 14 | 0 of 18 |
| changes | 18 | 10 | 15 | 0 of 18 |
| lint: verdict | 18 | 9 | 13 | 0 of 18 |
| gate | 18 | 9 | 12 | 3 of 19 |
| documentation: verdict | 18 | 8 | 14 | 0 of 18 |
| security: verdict | 18 | 6 | 9 | 0 of 18 |

### affected: 26 runs

| per run | median | p90 | min | max |
|---|---|---|---|---|
| wall clock (s) | 222 | 263 | 43 | 412 |
| of which waiting before the first job (s) | 3 | 3 | 2 | 149 |
| execution span (s) | 217 | 260 | 40 | 274 |
| runner-seconds (every execution) | 359 | 690 | 33 | 702 |
| of which in jobs that failed (s) | 0 | 0 | 0 | 116 |

0 of 26 runs needed a re-run (more than one attempt).

| job | runs | median (s) | p90 (s) | failed of executions, every attempt |
|---|---|---|---|---|
| **test, all 4 jobs (sum per run)** | 21 | 341 | 503 | |
| test: pg-adapter | 12 | 164 | 202 | 0 of 12 |
| test: ci-scripts | 21 | 154 | 217 | 2 of 21 |
| test: hermetic | 8 | 138 | 182 | 0 of 8 |
| **lint, all 5 jobs (sum per run)** | 19 | 117 | 163 | |
| lint: deps | 17 | 84 | 94 | 0 of 17 |
| lint: lint | 8 | 62 | 67 | 0 of 8 |
| lint: bash | 2 | 34 | 46 | 0 of 2 |
| **documentation, all 2 jobs (sum per run)** | 12 | 21 | 24 | |
| changes | 26 | 13 | 17 | 0 of 26 |
| documentation: docs | 12 | 12 | 15 | 0 of 12 |
| lint: yaml | 9 | 11 | 27 | 0 of 9 |
| lint: verdict | 19 | 11 | 15 | 0 of 19 |
| test: verdict | 21 | 11 | 13 | 2 of 21 |
| documentation: verdict | 12 | 9 | 12 | 0 of 12 |
| gate | 26 | 9 | 12 | 2 of 26 |

### runs measured

- full: 45319916322 (schedule), 45317437908 (pull_request), 45312968236 (pull_request), 45254852716 (pull_request), 45254576916 (push), 45251789156 (pull_request), 45250144054 (pull_request), 45244314229 (pull_request), 45241238157 (pull_request), 45234601898 (pull_request), 45134906524 (pull_request), 45134627967 (push), 45129116169 (pull_request), 45127786290 (pull_request), 45127576220 (push), 45126213584 (pull_request), 45125926424 (pull_request), 45125725107 (push)
- affected: 45327985074 (pull_request), 45327762211 (push), 45324833222 (pull_request), 45324042658 (pull_request), 45320496829 (pull_request), 45320359744 (pull_request), 45319311711 (pull_request), 45318749571 (pull_request), 45317213275 (push), 45317036454 (push), 45316703713 (pull_request), 45314771091 (pull_request), 45312802081 (push), 45252206478 (pull_request), 45249912732 (push), 45249238306 (pull_request), 45249152384 (push), 45248528301 (pull_request), 45244109632 (push), 45243460790 (pull_request), 45241006531 (push), 45236484908 (pull_request), 45234349567 (push), 45233563002 (pull_request), 45134602825 (push), 45128108874 (pull_request)

## Appendix B: the output of `full-runs.py`

```
run          event          att  wall(s)  e2e(s)  longest queue wait (s), the job
45251789156  pull_request     2      141     687       2  test / contract (worker-memory, csrfoff) / worker-memory csrfoff
45319916322  schedule         1      568     531      28  test / contract (native-pg, panic) / native-pg panic
45127576220  push             1      571     528      30  test / contract (worker-memory, panic) / worker-memory panic
45254576916  push             1      571     532      55  test / contract (worker-cf, oracle) / worker-cf oracle
45134627967  push             1      593     526      54  test / contract (worker-memory, mail) / worker-memory mail
45234601898  pull_request     1      596     530      68  test / contract (worker-cf, panic) / worker-cf panic
45125725107  push             1      601     514     120  test / contract (native-memory, sweeper) / native-memory sweeper
45254852716  pull_request     1      640     528      99  test / contract (worker-cf, e2e-mail) / worker-cf e2e-mail
45317437908  pull_request     1      644     526      67  test / contract (worker-cf, worker) / worker-cf worker
45241238157  pull_request     1      662     550      72  test / contract (native-pg, sweeper) / native-pg sweeper
45312968236  pull_request     1      666     588      66  test / contract (native-pg, oracle) / native-pg oracle
45126213584  pull_request     1      671     544      90  test / contract (worker-memory, sweeper) / worker-memory sweeper
45134906524  pull_request     1      676     498     113  test / contract (worker-cf, e2e) / worker-cf e2e
45129116169  pull_request     1      680     638      27  test / contract (worker-cf, mail) / worker-cf mail
45125926424  pull_request     1      700     506     162  test / contract (native-memory, sweeper) / native-memory sweeper
45127786290  pull_request     1      719     667     106  test / contract (worker-memory, panic) / worker-memory panic
45244314229  pull_request     1      793     750      75  test / contract (native-pg, sweeper) / native-pg sweeper
45250144054  pull_request     1      814     712      66  test / contract (worker-cf, oracle) / worker-cf oracle
median wall 662 s over 18 runs
```

## Appendix C: the output of `extended-times.py`

57 runs of build-extended.yml, 2 on probe/ branches left out

| job | runs | min (s) | median (s) | max (s) | by event |
|---|---|---|---|---|---|
| bsd | 4 | 184 | 214 | 218 | pull_request 3, push 1 |
| bsd-cpa | 1 | 236 | 236 | 236 | pull_request 1 |
| bsd-vm | 2 | 264 | 357 | 450 | pull_request 2 |
| docker | 11 | 188 | 219 | 329 | pull_request 8, push 3 |
| macos | 44 | 114 | 167 | 249 | pull_request 13, push 26, schedule 1, workflow_dispatch 4 |
| windows | 35 | 133 | 208 | 310 | pull_request 12, push 21, workflow_dispatch 2 |

failed jobs (not probes):
  run 45319311424 (pull_request, ci/build-stage-bsd): bsd-cpa, 31 s
