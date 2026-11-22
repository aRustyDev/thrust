---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0072: `Scheduler` port — Cron Triggers adapter and an in-process/cron adapter

## Context and Problem Statement

`plane/celery.py` defines the beat schedule via `django_celery_beat` `DatabaseScheduler` **[S]**: `stack_email_notification` every 5 minutes;
`instance_traces` every 6 hours; and daily UTC jobs — 00:00 `hard_delete`, 01:00 `archive_and_close_old_issues`, 01:30 and 03:45
`delete_old_s3_link` (the same task is registered twice), 02:00 `delete_unuploaded_file_asset`, 02:30 `delete_api_logs`, 02:45
`delete_email_notification_logs`, 03:00 `delete_page_versions`, 03:15 `delete_issue_description_versions`, 03:30 `delete_webhook_logs`.
The target uses Cron Triggers on Cloudflare and a scheduler on self-hosted (ADR-0002; interview round 3).

## Decision Drivers

* Schedules declared once in code/config, rendered into either `wrangler` `triggers.crons` or a self-hosted scheduler.
* Scheduled handlers must be thin: *enqueue* work (fan-out per tenant/project), not do it inline (15-minute cap).
* Exactly-once-ish firing is not guaranteed by either; handlers must be idempotent.
* Note: Plane's DB-backed beat allows runtime schedule edits; we do not need that.

## Considered Options

* Option 1: Declarative schedule table in code → `Scheduler` port; CF adapter = Cron Triggers; self-hosted adapter = in-process scheduler with leader election (or system cron invoking an HTTP/CLI trigger)
* Option 2: Cron-triggered **Workflows** (`schedules` on the Workflow binding) for jobs that are workflow-shaped
* Option 3: DB-driven scheduler (django-celery-beat equivalent) with a ticking Worker/Cron every minute
* Option 4: DO alarms as per-tenant timers

## Decision Outcome

Chosen option: **Option 1**, with Option 2 allowed *per job* where the job is already a workflow, because it is the smallest portable surface
and matches the interview decision ("Cron Triggers" CF-side, swappable). Source: interview round 3.

```rust
// crates/thrust-core/src/schedules.rs — single source of truth (round 25: Rust; replaces packages/core/schedules.ts)
pub const SCHEDULES: &[Schedule] = &[
    Schedule { id: "email-digest", cron: "*/5 * * * *", job: Job::NotificationsStackEmail },
    Schedule { id: "hard-delete",  cron: "0 0 * * *",   job: Job::MaintenanceHardDelete },
    // …
];   // jobs enqueue to Queue/Workflow; the handler returns quickly
// A build script renders SCHEDULES into wrangler `triggers.crons` (CF) or the in-process scheduler table (self-hosted).
```

Dedup note: the duplicate `delete_old_s3_link` registration (01:30 and 03:45) should become a single entry unless Q-0072.1 says it was intentional.

### Consequences

* Good, because Cron Triggers: 5-field cron plus Quartz-like extensions, UTC only, 250 triggers per account on Workers Paid (5 on Free), scheduled
  handler wall-time limit 15 minutes (CF Workers limits/Cron Triggers docs). Plane's ~11 schedules fit easily.
* Good, because schedules are version-controlled (wrangler config replaces triggers on deploy) rather than living in a DB table.
* Bad, because no runtime schedule editing (acceptable; not a Plane feature).
* Bad, because the self-hosted scheduler needs singleton firing (leader election or external cron) to avoid N replicas each firing.

### Confirmation

* Contract test: advancing a fake clock fires each schedule exactly once per tick per deployment (self-hosted) / `wrangler dev --test-scheduled` hits
  the same handler (CF).
* Static check: every `schedules[].job` resolves to a registered job; cron expressions validated against both adapters' accepted syntax.

## Pros and Cons of the Options

### Option 1: Declarative table, two adapters

* Good, because simplest, reviewable. * Bad, because singleton firing on self-hosted is our problem.

### Option 2: Workflow `schedules`

* Good, because durable per run; Workers Paid cron-triggered instances get a one-hour cron concurrency budget; up to 100 schedules per account (CF Workflows limits).
* Bad, because ties scheduling to the workflow engine, and the self-hosted engine (ADR-0071) must then also schedule.

### Option 3: DB-driven scheduler

* Good, because runtime editable. * Bad, because Plane never exposes this; adds a polling tick and a table to maintain.

### Option 4: DO alarms

* Good, because per-tenant, precise timers (alarm handler wall time 15 min). * Bad, because tenant-fan-out DO management overhead; overkill for global daily jobs (good fit later for recurring work items, F-E18).

## Update — interview round 20 (2026-10-18)

**Single cleanup schedule** (the duplicate `delete_old_s3_link` run is dropped, Q-0072.1); the **`instance_traces` telemetry job is dropped** (Q-0072.2).

## Update — review fixes, round 25 (2026-10-18)

The TypeScript schedule table is replaced by a Rust `const` table (ADR-0006). The in-process scheduler is part of the **P0 self-hosted adapter set** (needed for the outbox Cron sweeper, email digest and cleanups; SPEC R-P3). The cleanup jobs include the **P0 hard-delete purge at a 30-day default retention** (deliberate deviation from Plane's 60 days; ADR-0043, F-D10 now P0). `delete_old_s3_link` runs once; `instance_traces` is dropped.

## Update — spike results, round 26 (2026-10-18)

Spike 0001: the `scheduled` handler is native in workers-rs (`#[event(scheduled)]`) and runs the outbox sweeper `[tested]` through wrangler's `--test-scheduled` endpoint (nothing delivered before the call, delivered after). **An actual cron trigger firing is unverified** (Stage 2 step S2-4). The sweeper pass also ran on both native targets, where it is reached through `POST /internal/sweep`.

## Update — issue #59, the daily hard delete (2026-11-20)

The `SCHEDULES` table is built, in `crates/core/usecases/src/schedules.rs` (the path in the sketch above predates ADR-0009), with the two
schedules there are today: `outbox-sweep` (`* * * * *`) and `hard-delete` (`0 0 * * *`, F-D10, D-001). Where the build differs from the sketch:

- **No build script.** The crons are written in the wrangler files that carry triggers (`wrangler.toml`, `wrangler.deploy.toml`,
  `wrangler.stage2.toml`, `wrangler.pg.stage.toml`; `wrangler.memory.toml` and `wrangler.measure.toml` have none, as a test fires a cron at them
  through `--test-scheduled`), and `.github/scripts/tests/deploy_parity.py` reads the `cron: "…"` literals of the table and requires each
  file's `triggers.crons` to be exactly those, none missing, none extra, none twice (`--crons` for the two Stage files, which nothing else
  compares). A trigger the table lacks, or a schedule no file triggers, fails CI's `ci-scripts`; `test-deploy-template.sh` shows each way it
  can be wrong, including a cron added to the files and not to the code. That is the "static check" of Confirmation, for the triggers; the
  "every job resolves" half is the compiler's, since `Schedule::job` is an enum.
- **One place runs a job.** `run_cron(app, cron)` finds the schedule whose cron is that string, written the same, and runs its job
  (`run_job`). The Worker's `scheduled` handler passes it `event.cron()` (it used to sweep whatever fired); the server's timers call
  `run_job`. A cron the table does not have runs nothing and is logged, so a trigger nobody wired is a line in the log and not a second
  sweep. `mutant_cron_ignored` (a handler that reads no cron) is killed by `thrust-core/tests/schedules.rs` and, on the Worker, by
  `worker_cron_scheduled_handler_runs_the_hard_delete`.
- **A job does not enqueue.** `Job::HardDelete` (the sketch called it `MaintenanceHardDelete`) runs its bounded batches inline: at most 50
  batches of 100 work items, with the attachments under them, and 100 batches of 1,000 outbox rows, then it says `complete: false` and the next
  day carries on. A backlog is drained over days. That fits the handler's 15 minutes with room, and a queue would add a producer and a
  consumer for a job that must not run twice at once.
- **Self-hosted.** The in-process scheduler is, for this job, a task that sleeps to the next 00:00 UTC strictly after the last it ran for
  (`next_utc_midnight_after`), so a wake a moment early cannot run a day twice, and then runs `Job::HardDelete`. It has no leader election,
  which the Consequences above ask for: ADR-0070 (round 29) has one process hold the database. A day missed while the process was down is not
  made up, because the rule is by age and the next run deletes what it would have.
- **Not verified:** that a real `0 0 * * *` trigger fires on a deployed Worker. The handler is shown through `--test-scheduled` on both Worker
  targets, as the sweep's was in round 26; the every-minute trigger has a Stage 2 test of the real firing, and a daily one cannot be waited for
  the same way. Recorded as a gap, `docs/src/testing/status.md` G-50.

## Open Questions

* Q-0072.1 — **answered (round 20)**: single schedule.
* Q-0072.2 — **answered (round 20)**: `instance_traces` dropped.
* Q-0073 — consumer mapping (ADR-0073).

## More Information

* Plane: `apps/api/plane/celery.py`.
* CF: Cron Triggers <https://developers.cloudflare.com/workers/configuration/cron-triggers/>; limits <https://developers.cloudflare.com/workers/platform/limits/>.
* Related: [[ADR-0002]], [[ADR-0070]], [[ADR-0071]], [[ADR-0073]].
