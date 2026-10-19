---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0073: Notifications, email and webhook pipeline; mapping of Plane's background tasks

## Context and Problem Statement

In Plane, a write handler calls `issue_activity.delay(...)` (Celery); that task records `IssueActivity`, then fans out to `notifications`,
`model_activity`/`webhook_activity` (webhooks), versions, and so on; email goes through `email_notification_task.stack_email_notification`
(beat, every 5 min) which batches `EmailNotificationLog` rows into `send_email_notification` **[S]**. The target replaces this with a
transactional outbox (ADR-0046) feeding the `Queue` port (ADR-0070), `WorkflowEngine` (ADR-0071) and `Scheduler` (ADR-0072). This ADR fixes the
pipeline shape and maps every task. It is *proposed* because delivery guarantees, email provider and digest design are unresolved.

Inventory source: `apps/api/plane/bgtasks/*` (34 files, of which `__init__.py` and `apps.py` are not tasks → **32 task modules**) and `plane/license/bgtasks/tracer.py`; **49 `@shared_task` definitions** in total (verified by the round-25 review). The webhook HTTP sender `webhook_send_task` (decorated with arguments) was missing from the first grep-based list and is now in the table (Q-0073.5 answered).

## Decision Drivers

* No lost events (outbox) and no duplicate user-visible effects (idempotent consumers).
* Same pipeline on both adapter sets.
* Keep request handlers fast: derived work is async, but **read-your-writes** for the acting user's own activity feed.
* Email: Plane's 5-minute digest semantics; transactional mail (invites, magic codes) must be prompt.
* Webhooks: signed, retried, SSRF-guarded, logged with retention.

## Considered Options

* Option 1: **Outbox → single `events` topic → fan-out consumers** (activity, notifications, webhooks, search, versions), cron for digests and cleanups, Workflows for long jobs
* Option 2: **One queue per concern** produced directly by the use-case (no outbox; dual-write risk accepted)
* Option 3: **Synchronous in-request** for activity+notifications, async only for email/webhooks
* Option 4: **Event bus with per-subscriber queues** (topic exchange; RabbitMQ-style) emulated on CF with one queue per subscriber

## Decision Outcome

Decision pending — see **Q-0073**. Leading hypothesis: **Option 1** (confidence: medium–high) — matches ARCHITECTURE.md §2b, avoids dual-write, and one
topic keeps the port narrow; per-concern routing happens inside the consumer Worker (CF) or worker process (self-hosted) by event `type`.

### Task mapping (all rows are proposals for review)

Legend: **Q** queue consumer · **W** workflow · **C** cron (enqueues) · **S** synchronous (in-request/`waitUntil`) · **drop** not carried over.

| Plane module → task(s) | Today | Proposed | Notes |
|---|---|---|---|
| `issue_activities_task` → `issue_activity` | Celery task per write | **Q** (outbox `events`) | Core of the pipeline; acting user's feed read-your-writes — see Q-0073.6 |
| `notification_task` → `notifications` | Celery | **Q** | Derived from event; idempotent on `(event.id, receiver)` |
| `email_notification_task` → `stack_email_notification` | beat `*/5` | **C** → **Q** | Cron enqueues per-receiver digest messages |
| `email_notification_task` → `send_email_notification` | Celery | **Q** | Email port |
| `webhook_task` → `webhook_activity`, `model_activity` | Celery | **Q** | Fan-out to subscribers; retries with backoff; DLQ |
| `webhook_task` → **`webhook_send_task`** (`webhook_task.py:254`; `bind=True`, `autoretry_for`, `retry_backoff=600`, `max_retries=5`) | Celery | **Q** | The HTTP sender that owns the owner-chosen policy: **at-least-once, HMAC-signed, 5 retries with backoff, `WebhookLog` kept 30 days**; SSRF guard mandatory (R-S3, a **P0** deliverable) |
| `webhook_task` → `send_webhook_deactivation_email` | Celery | **Q** | Email port |
| `forgot_password_task` → `forgot_password` | Celery | **Q** (priority) | Transactional email; could be `S` via Email port with fallback to Q |
| `magic_link_code_task` → `magic_link` | Celery | **Q** (priority) / **S** | Latency-sensitive login |
| `user_email_update_task` → `send_email_update_magic_code`, `send_email_update_confirmation` | Celery | **Q** | |
| `workspace_invitation_task` → `workspace_invitation` | Celery | **Q** | |
| `project_invitation_task` → `project_invitation` | Celery | **Q** | |
| `project_add_user_email_task` → `project_add_user_email` | Celery | **Q** | |
| `user_activation_email_task` → `user_activation_email` | Celery | **Q** | |
| `user_deactivation_email_task` → `user_deactivation_email` | Celery | **Q** | |
| `issue_description_version_task` → `issue_description_version_task` | Celery | **Q** | Version capture from event |
| `issue_version_sync` → `issue_task` | Celery | **Q** | Version capture |
| `issue_version_sync` → `sync_issue_version`, `schedule_issue_version` | self-re-queuing backfill (`countdown=300`) | **W** one-off / **drop** | Data backfill; only needed when importing legacy data (Q-G3) |
| `issue_description_version_sync` → `sync_issue_description_version`, `schedule_issue_description_version` | self-re-queuing backfill | **W** one-off / **drop** | Same |
| `page_version_task` → `track_page_version` | Celery | **Q** (`PageVersionDue`, ADR-0061) | |
| `page_transaction_task` → `page_transaction` | Celery | **Q** | Mention diff on page save |
| `copy_s3_object` → `copy_s3_objects_of_description_and_assets` | Celery | **Q** (or **W** if large) | Blob port copy |
| `storage_metadata_task` → `get_asset_object_metadata` | Celery | **Q** (R2 event notification → queue) | Blob port |
| `file_asset_task` → `delete_unuploaded_file_asset` | beat 02:00 | **C** | Blob port delete |
| `exporter_expired_task` → `delete_old_s3_link` | beat 01:30 **and** 03:45 | **C** (single entry) | Q-0072.1 |
| `export_task` → `issue_export_task` | Celery | **W** | Stream to `BlobStore`; see ADR-0081 |
| `analytic_plot_export` → `analytic_export_task`, `export_analytics_to_csv_email` | Celery | **W** then **Q** (email) | |
| `deletion_task` → `soft_delete_related_objects`, `restore_related_objects` | Celery | **Q** (cascade, chunked) or **S** within UoW | Depends on Data ADRs (0040/0043); cascade size drives choice |
| `deletion_task` → `hard_delete` | beat 00:00 | **C** → **Q** chunks | **P0** (F-D10 moved to P0, round 25); retention default **30 days**, a deliberate deviation from Plane's 60 (`HARD_DELETE_AFTER_DAYS`); Plane's `hard_delete` covers an explicit list of ~18 models, not all tables (ADR-0043) |
| `issue_automation_task` → `archive_and_close_old_issues` | beat 01:00 | **C** → **Q** per project | F-B17 |
| `cleanup_task` → `delete_api_logs`, `delete_email_notification_logs`, `delete_page_versions`, `delete_issue_description_versions`, `delete_webhook_logs` | beat 02:30–03:30 | **C** (each chunked via **Q**) | Retention policy as config |
| `work_item_link_task` → `crawl_work_item_link_title` | Celery | **Q** | SSRF guard mandatory (Q-0141) |
| `recent_visited_task` → `recent_visited_task` | Celery | **S** (`waitUntil`) or **Q** | Low value; fire-and-forget |
| `event_tracking_task` → `track_event` | Celery (PostHog) | **drop** | PostHog dropped (round 18, ADR-0130); F-D12 telemetry is P2 opt-in via OTLP only |
| `logger_task` → `process_logs` | Celery (API logs → mongo/db) | **Q** or log-pipeline | Becomes observability concern (ADR-0130); Q-0073.7 |
| `workspace_seed_task` → `workspace_seed` | Celery | **W** (or **Q**) | One-off per new workspace |
| `dummy_data_task` → `create_dummy_data` | Celery | **drop** (dev tool) | Replace with seed script |
| `license/bgtasks/tracer.py` → `instance_traces` | beat every 6 h | **drop** | Dropped (round 20, Q-0072.2) |

### Consequences

* Good, because every Plane task has a destination and the cron list (ADR-0072) becomes the only scheduler surface.
* Good, because webhook/notification fan-out are consumers of one event stream, so adding Slack/GitHub sync (ADR-0085) is another consumer.
* Bad, because outbox + at-least-once pushes idempotency onto each consumer (dedupe by `event.id`).
* Neutral: ~14 of the 49 tasks are single-purpose transactional emails; a templated `EmailRequested` event may collapse them into one consumer.

### Confirmation

* Static test: every task in this table has an owner module and a registered consumer/cron/workflow; unregistered ⇒ CI failure.
* Contract tests: replaying the same event twice produces one notification, one webhook delivery per subscriber (attempt rows may repeat), one email.

## Pros and Cons of the Options

### Option 1: Outbox + single topic + fan-out

* Good, because atomic with the write; replayable; one port topic. * Bad, because consumers share a queue (noisy neighbor) unless split later; per-subscriber retry isolation is by consumer logic.

### Option 2: Queue per concern, direct produce

* Good, because simplest and isolates retries. * Bad, because dual-write: a crash between commit and publish drops events (the exact failure the outbox prevents).

### Option 3: Synchronous activity/notifications

* Good, because read-your-writes trivial. * Bad, because write latency grows with fan-out; hostile to Workers CPU/subrequest limits.

### Option 4: Per-subscriber queues

* Good, because retry isolation and independent scaling. * Bad, because more queues to provision/manage on CF; topic exchange semantics don't exist natively there.

## Update — interview round 7 (2026-10-17): email delivery

Email delivery decided: **Cloudflare Email Service on the Cloudflare adapter set; plain SMTP adapter for self-hosted/other providers**, both behind the `Email` port (Q-0073.1 answered). Email is P0 (F-D02 digest; password verification/reset mail for the P0 user+password login). The rest of this ADR (queue/workflow/cron mapping of the bgtasks modules, webhook guarantees) remains `proposed`.

## Update — interview round 17 (2026-10-17)

Webhooks: **at-least-once, HMAC-signed, 5 retries with backoff, delivery logs kept 30 days** (Q-0073.2). Exports: **CSV/JSON via a Workflow, links valid 24 h** (Q-0081.2, Q-0081.3); exports are P1, size ceilings set then (start at 100 MB; Q-0073.3). Self-hosted durability: Postgres-backed runner (see ADR-0071).

## Update — interview round 20 (2026-10-18)

API request logging (`APIActivityLog`) **moves to observability** (Q-0073.7).

## Update — review fixes, round 25 (2026-10-18)

Task table corrected against the source: `webhook_send_task` added; `instance_traces` and `track_event` marked **drop** to match rounds 20 and 18; module/task counts fixed (32 modules, 49 `@shared_task`). The hard-delete row is P0 with a 30-day default (deviation recorded in ADR-0043). The pipeline decision itself is unchanged and the ADR stays `proposed`.

## Open Questions

* Q-0073 — pipeline shape and task mapping sign-off.
* Q-0073.1 — email provider: Cloudflare Email Service vs SMTP vs transactional provider; digest design (5-min stacking).
* Q-0073.2 — webhook delivery guarantees (retry schedule, signing, `WebhookLog` retention).
* Q-0073.5 — **answered (round 25 review)**: inventory completed (`webhook_send_task` added; 32 task modules, 49 `@shared_task`s). Re-check `plane/app` and `plane/api` for non-`bgtasks` tasks during planning.
* Q-0073.6 — read-your-writes for the acting user's activity/notification counts: synchronous write of the activity row in the same transaction, or accept lag?
* Q-0073.7 — **answered (round 20)**: move API request logging to observability.

## More Information

* Plane: `apps/api/plane/bgtasks/`, `plane/celery.py`, `plane/license/bgtasks/tracer.py`; Open-EE `[O] f0-event-backbone`, `slack-integration`, `external-sync`.
* Related: [[ADR-0046]], [[ADR-0070]], [[ADR-0071]], [[ADR-0072]], [[ADR-0081]], [[ADR-0085]], [[ADR-0130]].
