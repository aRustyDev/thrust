---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0046: Event / outbox contract

## Context and Problem Statement

Plane fans out from entity mutations through three overlapping mechanisms **[S][O]**: (1) `model_activity.delay(...)` → `webhook_activity` → `webhook_send_task` (HMAC-SHA256 signing, SSRF-safe `pinned_fetch`, retries/backoff, auto-deactivate, `WebhookLog`); (2) `issue_activity` Celery task diffing old/new and writing `IssueActivity` rows plus notifications/email stacking; (3) assorted bgtasks (versions, search, deletion). There is **no internal subscriber bus**. The Open-EE `f0-event-backbone` proposed an `EventOutbox` table + dispatcher + in-process subscribers (Audit, Slack, Intake, external Sync, Recurring) with at-least-once delivery and idempotency via delivery id / `external_id` **[O]**. Because the Open-EE program is superseded (ADR-0004), this ADR defines the contract natively and must work with the `Queue` port (ADR-0070) on both adapter sets. Bulk operations bypass `Issue.save()`, so emission cannot rely on model hooks **[S]**.

## Decision Drivers

* No lost events when a mutation commits; no phantom events when it rolls back.
* At-least-once with idempotent consumers; ordering per entity.
* One event schema consumed by activity log, notifications, webhooks, audit, search indexing, integrations.
* Works with Queues (CF) and RabbitMQ (self-hosted) via the `Queue` port.
* Bulk paths must emit explicitly (there is no import path, Q-G3).

## Considered Options

* Option 1: Transactional outbox table written in the same transaction; a relay publishes to the `Queue` port; consumers idempotent
* Option 2: Direct enqueue to the `Queue` port after commit (dual write, no outbox)
* Option 3: Change-data-capture (Postgres logical decoding / DB triggers) producing events
* Option 4: Event sourcing for selected aggregates (events as the source of truth)

## Decision Outcome

**Decision (accepted, round 29): as built in PR #17, Option 1: envelope v1 as in `thrust-contracts`, written to the outbox in the entity's transaction; an after-commit relay plus a cron sweeper; outbox rows kept 90 days; consumers idempotent on `event_id`. Per FR-17, `reviews/2026-10-19-fable-architecture-review.md` §5.**

**Option 1** is chosen because it is the only option that gives atomicity across (entity write + event) on every DB engine in ADR-0040 and maps directly to the Open-EE design already reviewed. Relay: after-commit fast path (`waitUntil`/in-process publish) plus a Cron-driven sweeper for undispatched rows (Q-0046.1, answered round 16).

### Proposed envelope (schema v1 as first written; the accepted v1 is in the round-29 note)

```json
{
  "event_id": "uuid",              // dedupe key for consumers
  "schema_version": 1,
  "workspace_id": "uuid",          // tenant key (ADR-0041)
  "event_type": "issue.updated",   // <entity>.<verb>
  "entity_type": "issue",
  "entity_id": "uuid",
  "actor_id": "uuid|null",
  "occurred_at": "RFC3339",
  "payload": { "new": {...}, "old": {...}, "changed_fields": ["state_id"] },
  "trace_id": "w3c traceparent",   // ADR-0130
  "external_source": "string|null",
  "external_id": "string|null"     // integration idempotency, as on Issue/State/IntakeIssue [S]
}
```

Outbox row adds `dispatched_at`, `attempts`, `dead_lettered_at`. Ordering guarantee: per `(workspace_id, entity_id)` — **mechanism** (round 25): the relay sets `ordering_key = (workspace_id, entity_id)` on the `Queue` port (ADR-0070); Cloudflare Queues has no ordering primitive, so the CF adapter uses **single-consumer-per-key** (a Durable Object that serialises keyed messages, or a per-entity sequence number the consumer checks and retries on gap) and RabbitMQ uses a consistent-hash exchange or single-active-consumer. Un-keyed events carry no ordering guarantee.

### Consumers (all idempotent on `event_id`)

| Consumer | Replaces in Plane |
|---|---|
| Activity writer | `issue_activity` → `IssueActivity` rows (verb/field/old/new) |
| Notification builder | notification + `stack_email_notification` inputs |
| Webhook dispatcher | `model_activity`/`webhook_send_task` (keep HMAC, SSRF guard, retries, log) |
| Audit log (Open-EE F-E06) | none in CE |
| Search indexer | n/a (ADR-0083) |
| Integrations (Slack, GitHub/Beads) | Open-EE `slack-integration`, `external-sync` |

### Consequences

* Good: durable, atomic, single contract; new consumers need no producer changes.
* Bad: outbox table growth (needs retention job; fits existing daily cleanups); additional write per mutation; relay latency; consumer idempotency burden.
* Neutral: `IssueActivity` becomes a *projection*, written by a consumer (eventually consistent), as built (round 29; Q-0046).

### Confirmation

Contract tests (ADR-0121): atomic rollback leaves no event; relay crash redelivers; duplicate delivery processed once; per-entity ordering. Spike S6 (ADR-0040). Lint: all repositories' write methods that mutate tracked entities require `tx.emit(...)` or an explicit `noEvent` justification (bulk paths included).

## Pros and Cons of the Options

### Option 1: Transactional outbox

* Good: atomic; engine-neutral; auditable; replayable.
* Bad: extra table/relay; at-least-once only.

### Option 2: Direct enqueue after commit

* Good: simplest.
* Bad: dual-write — crash after commit loses events (webhooks/audit silently missing); unacceptable for audit.

### Option 3: CDC

* Good: no producer code; captures bulk paths.
* Bad: Postgres-only (logical decoding) or trigger-heavy; unavailable on D1/DO; payloads lack actor/intent context.

### Option 4: Event sourcing

* Good: perfect history; natural audit.
* Bad: enormous departure from Plane's CRUD model and API semantics; rewriting every aggregate.

## Update — interview round 16 (2026-10-17)

Outbox relay: **immediate publish after commit plus a Cron sweeper**; events kept **90 days**; no replay API at P0 (Q-0046.1, Q-0046.2). The acting user's activity feed reads the DB so they see their own writes (Q-0073.6). Contract shape (Q-0046) still open.

## Update — review fixes, round 25 (2026-10-18)

Envelope message size must stay under the Queue common-subset ceiling of **128 KB** (ADR-0070; Q-0070.1 closed) — large `payload.old/new` diffs carry changed fields and IDs, not full bodies. The Rust representation is a `serde`-serialised struct in `thrust-ports`; the JSON above is the wire schema. Outbox retention 90 days, no replay API at P0 (round 16). Import-era wording removed (Q-G3). The ADR stays `proposed` (contract shape still open).

## Update — spike results, round 26 (2026-10-18)

Spike 0001: both delivery paths work on both targets `[tested]` locally — the after-commit fast path (`Background` port: `waitUntil` on the Worker, `tokio::spawn` natively) and the Cron-driven sweeper (an outbox row nothing delivered before the sweep is delivered by it). The consumer writes the activity row idempotently on `event_id`, verified with an injected duplicate delivery. Cloudflare Queues provides no per-key ordering (unchanged; see ADR-0070). Queue messages are JSON text. Retry, DLQ and real delivery semantics are Stage 2.

## Update — review round 29 (2026-10-19)

Per FR-17 (`reviews/2026-10-19-fable-architecture-review.md`), this ADR is accepted as built in PR #17 (owner disposition in review §5), and Q-0046 is answered. The review checked the design and found its at-least-once semantics correct (review §4, "Transactional outbox"). As built:

* **Envelope v1** is `Envelope` in `thrust-contracts` (`crates/thrust-contracts/src/lib.rs:125-139`), not in `thrust-ports` as the round-25 note says. Its fields: `event_id`, `schema_version` (1, `crates/thrust-core/src/lib.rs:209`), `workspace_id`, `event_type` (`<entity>.<verb>`), `entity_type`, `entity_id`, `actor_id`, `occurred_at`, `payload`, `trace_id`. The proposed JSON's `external_source` and `external_id` are not in v1. The outbox row (`crates/thrust-contracts/src/lib.rs:150-157`; table at `crates/thrust-adapters-postgres/migrations/0001_init.sql:53-62`) carries `dispatched_at` and `attempts`; `dead_lettered_at` is not built.
* **Atomic write.** The entity row and the outbox row are written in one transaction, with an explicit rollback on error (`crates/thrust-core/src/lib.rs:250-298`).
* **Relay.** After commit, `Background::spawn` publishes the row and marks it dispatched; a publish failure leaves the row for the sweeper (`crates/thrust-core/src/lib.rs:380-410`). The sweeper (`sweep_outbox`, lines 419-430) runs from the Worker's cron every minute (`crates/thrust-worker/wrangler.toml:21`, `crates/thrust-worker/src/lib.rs:193-211`) and from an in-process timer natively (`crates/thrust-server/src/main.rs:93-106`). The relay sets the event id as the idempotency key and `ordering_key` to `<workspace_id>:<entity_id>` (line 389); no queue adapter reads `ordering_key` yet, so per-key ordering stays with ADR-0070.
* **Consumer idempotency.** `process_event` (lines 432-454) writes the activity row with `ON CONFLICT (event_id) DO NOTHING` against a unique `event_id` (`crates/thrust-adapters-postgres/src/lib.rs:253-258`; `0001_init.sql:64-67`), and the `no-activity-dedupe` mutation control (`scripts/mutants.sh:32`) is a broken build the suite must fail. The activity row is a consumer projection, which settles the Neutral consequence above.
* **Retention.** Outbox rows are kept 90 days (round 16). The retention job is not built yet.

## Open Questions

* Q-0046 — approve outbox + envelope; activity as sync write vs consumer projection. (answered round 29, FR-17)
* Q-0073 — mapping of bgtasks to consumers/workflows.
* Q-0073.2 — webhook delivery guarantees.
* Q-0046.1 — Relay mechanism: immediate publish + Cron sweeper vs sweeper-only (latency vs complexity)?
* Q-0046.2 — Event retention/archival and replay policy.

## More Information

[[ADR-0004]], [[ADR-0045]], [[ADR-0070]], [[ADR-0073]]. Open-EE: `woven/plan:.claude/plans/plane-fork/features/f0-event-backbone.md`, `research/integrations-workflow.md`. Plane: `apps/api/plane/bgtasks/{webhook_task,issue_activities_task}.py`, `db/models/webhook.py`.
