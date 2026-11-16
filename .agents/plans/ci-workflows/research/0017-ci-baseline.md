# Research 0017: what `ci` costs today (the baseline)

Stage 0.5 of [phase 0001](../phases/0001-foundation.md), part of [ci-workflows](../PLAN.md). Collected 2026-11-16 21:02 UTC by
[`scripts/ci-baseline.py`](../scripts/ci-baseline.py) from GitHub's run and job records of `ci.yml`. It is what the passes
that save time (5 to 7) are measured against, and what the migration (Phase 1) must not change: the same measurement is
taken again after it.

## What was measured

**Window.** Runs created since 2026-11-16T15:51:24Z, when #206 merged. That was the last change to `ci.yml`'s job set: the
ten jobs, `changes` and `gate` are the same in `ci.yml` at `95bc954` (#206) and at `b8702d3`. An older run has other jobs (a
`plan` job, no `docs`) and is not comparable; the script refuses one (`--groups` must list every job group it finds).
33 runs fall in the window: 6 were cancelled (a superseded pull-request run measures nothing), 4 planned no job, 5 were
**affected-only** and 18 were **full**.

**The sample is small.** The window is about five hours of one busy day. The 18 full runs are usable; the 5 affected-only
runs are anecdotal (most job rows hold one run), and a day with many CI changes is not a typical day. Run the script again
later with the same `--since`; the window stays valid until `ci.yml`'s job set changes.

**Method.** Each point below was probed on 2026-11-16 and is in the script's header and self-test.
- *Full* means every job group ran, whatever started the run (an `rc/*` pull request, a push that changed `Cargo.lock`, the
  nightly run); *affected-only* means the planner skipped some; *nothing* means only `changes` and `gate` ran.
- **Cost is runner-seconds**, the sum of `completed_at - started_at` over every job execution. GitHub reports 0 billable
  minutes for this public repository (`gh api .../actions/runs/<id>/timing`).
- **An execution is a job name and its start time.** GitHub lists a job that passed in attempt 1 again under attempts 2
  and 3, with a new id and its original times (run 43902442880). So a re-run's failed attempts are counted, as the wasted
  work they are, and a carried-over job is not counted twice.
- **Wall clock is the final attempt's**: `run_started_at` to the last completion, split into the wait before the first job
  started and the execution span after it. It matches GitHub's own `run_duration_ms` to within a second on the three
  runs compared: a one-attempt run (1136 s mine, 1136 s GitHub's), a run that queued (703 s, 704 s) and a run with three
  attempts (925 s, 926 s).
- A job's duration is its final entry's, the execution that decided the result. A skipped job counts for nothing (GitHub
  stamps it with a start one second after its end).
- Pages of 30 jobs, not 100: for a run with three attempts (108 jobs) GitHub answered every `per_page=100` request with HTTP
  502 and `per_page=30` with the jobs.

## What the numbers say

Observations for the passes that follow, not decisions. Figures are medians of the 18 full runs unless said.

- **A full run takes 766 s of wall clock** (p90 1136 s) and **4552 runner-seconds** (76 minutes; p90 5269 s). The execution
  span, 750 s, is close to `changes`, `mutants` and `gate` in series (10 + 692 + 10 s): the longest job decides the wall
  clock. `mutants` (692 s) and `contract: worker-cf e2e` (622 s) are the two longest.
- **The 25 contract cells are 70% of the runner time** (3198 of 4552 s); `mutants` is 15%. A change that makes the cells
  cheaper pays most in pass 7; one that shortens `mutants` and the e2e cell pays most in pass 6.
- **Failures cost real time.** `mutants` failed in 8 of its 22 executions, `gate` in 10 of 23 (a gate fails when a job
  before it does). In a full run, the time spent in jobs that failed has a median of 74 s, a p90 of 883 s and a maximum of
  1825 s. 4 of the 18 full runs needed a re-run (none of the 5 affected-only ones), and the worst full run cost 6613
  runner-seconds, 45% above the median.
- **A run can wait before it starts.** The wait before the first job has a median of 4 s in full runs, but a p90 of 319 s and
  a maximum of 425 s, and in the "nothing" class a p90 of 682 s. One case was checked: run 43947826462 (the push that merged
  #225) was created at 20:15:09Z, and its first job started at 20:26:31Z, two seconds after the previous `main` run,
  43947772104, finished at 20:26:29Z. `ci.yml` never cancels a push run, and groups them by ref, so runs on `main` queue
  one behind the other. The other long waits fit the same cause; only this one was checked. A pass that cuts the length of a
  run on `main` cuts this wait too.
- **Wall clock is not cost.** A "nothing" run (`changes` and `gate` only) has a median wall clock of 39 s, and a p90 of 703 s
  that is almost all waiting (682 s of it): the run used 19 runner-seconds.

## To repeat it

```sh
uv run --script .agents/plans/ci-workflows/scripts/ci-baseline.py --since 2026-11-16T15:51:24Z
uv run --script .agents/plans/ci-workflows/scripts/ci-baseline.py --self-test     # 38 checks, no network
```

- It needs `gh`, authenticated, and prints the tables below to stdout; it exits 2, and prints no table, when it cannot
  trust the data (no run to measure, a run with no jobs, a job with no conclusion, a negative duration, an unknown job group,
  a malformed `--since`, a failed request).
- The self-test is the control: it checks the arithmetic on a hand-worked example and that each of those refusals refuses.
  Eleven bugs planted one at a time (no dedupe of carried-over jobs, a wrong p90, an accepted malformed `--since`, a 404
  retried, and others) were each caught by it.
- After the migration the job names change (`test / hermetic`, not `hermetic`). Pass the new names to `--groups`; until then
  the script stops at the first run it cannot classify, which is the guard working.

## The script's output, unedited

Collected 2026-11-16 21:02 UTC: 33 runs scanned, 18 full and 5 affected-only measured, 4 planned no job (only `changes` and `gate`), excluded: 6 cancelled.

### full: 18 runs

| per run | median | p90 | min | max |
|---|---|---|---|---|
| wall clock (s) | 766 | 1136 | 183 | 1301 |
| of which waiting before the first job (s) | 4 | 319 | 2 | 425 |
| execution span (s) | 750 | 919 | 178 | 938 |
| runner-seconds (every execution) | 4552 | 5269 | 4223 | 6613 |
| of which in jobs that failed (s) | 74 | 883 | 0 | 1825 |

4 of 18 runs needed a re-run (more than one attempt).

| job | runs | median (s) | p90 (s) | failed of executions, every attempt |
|---|---|---|---|---|
| **contract, all 25 jobs (sum per run)** | 18 | 3198 | 3288 | |
| mutants | 18 | 692 | 868 | 8 of 22 |
| contract: worker-cf e2e | 18 | 622 | 711 | 0 of 18 |
| contract: worker-cf oracle | 18 | 212 | 242 | 0 of 18 |
| contract: worker-cf main | 18 | 176 | 186 | 1 of 19 |
| hermetic | 18 | 162 | 195 | 1 of 18 |
| contract: worker-cf sweeper | 18 | 148 | 164 | 0 of 18 |
| contract: native-pg oracle | 18 | 146 | 150 | 0 of 18 |
| contract: worker-cf worker | 18 | 140 | 148 | 0 of 18 |
| contract: worker-cf panic | 18 | 129 | 142 | 1 of 19 |
| contract: worker-cf mail | 18 | 125 | 138 | 0 of 18 |
| pg-adapter | 18 | 123 | 154 | 1 of 18 |
| contract: worker-cf cron | 18 | 122 | 133 | 0 of 18 |
| contract: worker-cf bindings | 18 | 118 | 134 | 0 of 18 |
| contract: worker-memory main | 18 | 112 | 123 | 0 of 18 |
| contract: native-memory oracle | 18 | 110 | 126 | 0 of 18 |
| contract: worker-memory csrfoff | 18 | 105 | 115 | 0 of 18 |
| contract: worker-memory sweeper | 18 | 104 | 119 | 0 of 18 |
| contract: worker-memory panic | 18 | 104 | 115 | 0 of 18 |
| contract: worker-memory mail | 18 | 97 | 111 | 0 of 18 |
| contract: worker-memory cron | 18 | 94 | 110 | 0 of 18 |
| contract: native-pg main | 18 | 88 | 99 | 0 of 18 |
| contract: native-pg sweeper | 18 | 82 | 95 | 0 of 18 |
| contract: native-pg panic | 18 | 78 | 93 | 0 of 18 |
| ci-scripts | 18 | 71 | 79 | 0 of 18 |
| deps | 18 | 69 | 80 | 0 of 18 |
| web | 18 | 62 | 79 | 0 of 18 |
| contract: native-memory mail | 18 | 58 | 74 | 0 of 18 |
| lint | 18 | 56 | 68 | 1 of 18 |
| contract: native-memory main | 18 | 56 | 72 | 0 of 18 |
| contract: native-memory sweeper | 18 | 53 | 69 | 0 of 18 |
| contract: native-memory csrfoff | 18 | 50 | 63 | 0 of 18 |
| contract: native-memory panic | 18 | 47 | 65 | 0 of 18 |
| deny | 18 | 28 | 37 | 1 of 18 |
| docs | 18 | 11 | 18 | 0 of 18 |
| changes | 18 | 10 | 18 | 0 of 18 |
| gate | 18 | 10 | 16 | 10 of 23 |

### affected: 5 runs

| per run | median | p90 | min | max |
|---|---|---|---|---|
| wall clock (s) | 374 | 1432 | 216 | 1432 |
| of which waiting before the first job (s) | 111 | 695 | 2 | 695 |
| execution span (s) | 237 | 746 | 100 | 746 |
| runner-seconds (every execution) | 691 | 4370 | 177 | 4370 |
| of which in jobs that failed (s) | 0 | 716 | 0 | 716 |

0 of 5 runs needed a re-run (more than one attempt).

| job | runs | median (s) | p90 (s) | failed of executions, every attempt |
|---|---|---|---|---|
| contract: worker-cf e2e | 1 | 720 | 720 | 0 of 1 |
| mutants | 1 | 703 | 703 | 1 of 1 |
| **contract, all 25 jobs (sum per run)** | 4 | 484 | 3274 | |
| contract: worker-cf oracle | 4 | 217 | 238 | 0 of 4 |
| contract: worker-cf main | 1 | 176 | 176 | 0 of 1 |
| hermetic | 4 | 149 | 186 | 1 of 4 |
| contract: worker-cf worker | 1 | 147 | 147 | 0 of 1 |
| contract: worker-cf sweeper | 1 | 145 | 145 | 0 of 1 |
| contract: native-pg oracle | 4 | 144 | 161 | 0 of 4 |
| contract: worker-cf cron | 1 | 134 | 134 | 0 of 1 |
| contract: worker-cf mail | 1 | 125 | 125 | 0 of 1 |
| contract: worker-cf panic | 1 | 121 | 121 | 0 of 1 |
| contract: worker-memory panic | 1 | 109 | 109 | 0 of 1 |
| contract: worker-memory main | 1 | 109 | 109 | 0 of 1 |
| contract: worker-cf bindings | 1 | 108 | 108 | 0 of 1 |
| contract: worker-memory sweeper | 1 | 106 | 106 | 0 of 1 |
| contract: native-memory oracle | 4 | 105 | 116 | 0 of 4 |
| contract: worker-memory mail | 1 | 103 | 103 | 0 of 1 |
| contract: worker-memory csrfoff | 1 | 97 | 97 | 0 of 1 |
| contract: native-pg main | 1 | 93 | 93 | 0 of 1 |
| contract: native-pg sweeper | 1 | 90 | 90 | 0 of 1 |
| contract: native-pg panic | 1 | 74 | 74 | 0 of 1 |
| ci-scripts | 2 | 74 | 81 | 0 of 2 |
| deps | 5 | 72 | 74 | 0 of 5 |
| contract: worker-memory cron | 1 | 72 | 72 | 0 of 1 |
| contract: native-memory main | 1 | 68 | 68 | 0 of 1 |
| lint | 1 | 64 | 64 | 0 of 1 |
| contract: native-memory csrfoff | 1 | 64 | 64 | 0 of 1 |
| contract: native-memory sweeper | 1 | 55 | 55 | 0 of 1 |
| contract: native-memory mail | 1 | 53 | 53 | 0 of 1 |
| contract: native-memory panic | 1 | 46 | 46 | 0 of 1 |
| docs | 5 | 18 | 21 | 0 of 5 |
| gate | 5 | 10 | 13 | 2 of 5 |
| changes | 5 | 8 | 12 | 0 of 5 |

### nothing: 4 runs

wall clock median 39 s, p90 703 s; of which waiting before the first job median 3 s, p90 682 s.

### runs measured

- full: 43952361628 (pull_request), 43952152076 (push), 43948186258 (pull_request), 43947772104 (push), 43946266175 (pull_request), 43945149717 (push), 43942106891 (pull_request), 43941895645 (pull_request), 43939613096 (push), 43925759039 (pull_request), 43921133682 (pull_request), 43919387399 (push), 43904442151 (push), 43902442880 (pull_request), 43902141066 (pull_request), 43901702757 (pull_request), 43901511193 (pull_request), 43900965904 (push)
- affected: 43941066070 (push), 43940351842 (pull_request), 43920414716 (pull_request), 43919557103 (push), 43901279568 (push)
