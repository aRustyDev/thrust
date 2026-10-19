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

## Open Questions

* Q-0072.1 — **answered (round 20)**: single schedule.
* Q-0072.2 — **answered (round 20)**: `instance_traces` dropped.
* Q-0073 — consumer mapping (ADR-0073).

## More Information

* Plane: `apps/api/plane/celery.py`.
* CF: Cron Triggers <https://developers.cloudflare.com/workers/configuration/cron-triggers/>; limits <https://developers.cloudflare.com/workers/platform/limits/>.
* Related: [[ADR-0002]], [[ADR-0070]], [[ADR-0071]], [[ADR-0073]].
