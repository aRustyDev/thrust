---
status: proposed
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

Decision pending — see Q-0046. Leading hypothesis: **Option 1**, confidence **medium-high**, because it is the only option that gives atomicity across (entity write + event) on every DB engine in ADR-0040 and maps directly to the Open-EE design already reviewed. Relay: after-commit fast path (`waitUntil`/in-process publish) plus a Cron-driven sweeper for undispatched rows (Q-0046.1, answered round 16).

### Proposed envelope (schema v1)

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
* Neutral: `IssueActivity` becomes a *projection* — decide whether it is written by a consumer (eventually consistent) or synchronously inside the transaction for UI read-your-writes (Q-0046).

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

## Open Questions

* Q-0046 — approve outbox + envelope; activity as sync write vs consumer projection.
* Q-0073 — mapping of bgtasks to consumers/workflows.
* Q-0073.2 — webhook delivery guarantees.
* Q-0046.1 — Relay mechanism: immediate publish + Cron sweeper vs sweeper-only (latency vs complexity)?
* Q-0046.2 — Event retention/archival and replay policy.

## More Information

[[ADR-0004]], [[ADR-0045]], [[ADR-0070]], [[ADR-0073]]. Open-EE: `woven/plan:.claude/plans/plane-fork/features/f0-event-backbone.md`, `research/integrations-workflow.md`. Plane: `apps/api/plane/bgtasks/{webhook_task,issue_activities_task}.py`, `db/models/webhook.py`.
