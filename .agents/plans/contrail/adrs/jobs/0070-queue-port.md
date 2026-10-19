---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0070: `Queue` port with Cloudflare Queues and RabbitMQ adapters

## Context and Problem Statement

Plane dispatches 49 Celery `@shared_task`s (32 task modules) over RabbitMQ (with Redis as result/cache layer) from `apps/api/plane/bgtasks` **[S]**: activity
recording, notifications, webhooks, emails, versioning, link crawling. The target replaces Celery/RabbitMQ with Cloudflare Queues on the
CF adapter set, and must keep a self-hosted adapter (ADR-0002). The interview ([I] round 3) decided "Queues + Workflows + Cron Triggers"
for CF and portability via interfaces ("Queues vs RabbitMQ").

## Decision Drivers

* Same producer/consumer code on both adapter sets.
* At-least-once delivery with idempotent consumers (events come from the transactional outbox, ADR-0046).
* Retries with backoff, dead-letter handling, bounded concurrency.
* Message payloads must stay small (references, not blobs).
* Not a Celery-compat layer: avoid porting Celery semantics (ETA/countdown chains) 1:1.

## Considered Options

* Option 1: Narrow `Queue` port (publish, batch consume with ack/retry/delay, DLQ) with Cloudflare Queues and RabbitMQ adapters
* Option 2: Wide "Celery-compatible" task API (named tasks, `apply_async`, countdown, chains) with adapters
* Option 3: DB-backed queue (outbox table polled) on both sets, no broker
* Option 4: Cloudflare Queues only (no self-hosted adapter)

## Decision Outcome

Chosen option: **Option 1**, because it matches the interview decision, keeps the port small enough to implement twice, and pushes
workflow-shaped needs (long, multi-step, sleep/retry) to the Workflow port (ADR-0071) instead of re-creating Celery chains.
Source: interview round 3.

Port sketch:

```rust
// crate: thrust-ports (round 25: Rust sketch; replaces the earlier TypeScript sketch)
pub struct SendOpts { pub delay: Option<Duration>, pub idempotency_key: Option<String>, pub ordering_key: Option<OrderingKey> }

#[async_trait]                      // Send futures on every target (round 26, ADR-0002): Cloudflare adapters wrap JS futures in SendFuture
pub trait Queue: Send + Sync {
    async fn send(&self, topic: Topic, msg: Envelope, opts: SendOpts) -> Result<(), QueueError>;
    async fn send_batch(&self, topic: Topic, msgs: Vec<Envelope>) -> Result<(), QueueError>; // ≤100 msgs / 256 KB
}

#[async_trait]
pub trait QueueConsumer: Send + Sync {           // driving adapter registers handlers
    async fn handle(&self, batch: Vec<Delivery>) -> Result<(), ConsumerError>; // per-message ack()/retry(Duration)
}
```

`Envelope` carries `{id, type, tenantId, occurredAt, traceId, schemaVersion, payload}` (contract in ADR-0046).

### Consequences

* Good, because Cloudflare Queues provides batching (`max_batch_size`, `max_batch_timeout`), `max_retries`, `retry_delay`,
  `dead_letter_queue`, and `max_concurrency` as consumer settings (wrangler configuration docs); RabbitMQ has equivalents (prefetch,
  DLX, TTL).
* Good, because retention is configurable 60 s–14 days (default 4 days; 24 h on Workers Free, CF changelog) — adequate for outbox relays.
* Bad, because semantics differ at the edges (ordering, delay maximums, visibility); the contract suite must pin the common subset:
  **at-least-once, ordering only per `ordering_key` (see below), message ≤ 128 KB, batch ≤ 100 messages / 256 KB, delay ≤ 86,400 s (24 h, the Cloudflare maximum; RabbitMQ needs the delayed-message plugin or TTL + dead-letter exchange to match)**.
* **Ordering mechanism (round 25; owner: ordering per work item only, Q-0070.2).** The producer sets `ordering_key = (workspace_id, entity_id)`. Cloudflare Queues has no ordering primitive, so the CF adapter guarantees order by **single-consumer-per-key**: either routing keyed messages through a Durable Object that serialises them, or a **per-entity sequence number checked by the consumer** (stale/out-of-order messages are retried with a short delay). RabbitMQ uses a **consistent-hash exchange or single-active-consumer** per key. Un-keyed messages carry no ordering guarantee. The same rule is referenced from ADR-0046.
* Neutral: pull consumers exist for off-Cloudflare consumption (up to 5,000 msg/s per queue, batch ≤ 100, visibility timeout ≤ 12 h) —
  a possible bridge if a self-hosted worker must consume a CF queue.

### Confirmation

* Port contract suite (ADR-0121): publish/consume, retry→redelivery, DLQ after N failures, delayed delivery, idempotent re-delivery,
  against workerd/Miniflare Queues and RabbitMQ testcontainer.
* Lint rule (e.g. `cargo-deny` bans / a workspace dependency check): no direct Cloudflare `worker::Queue` or RabbitMQ client (`lapin`) imports outside adapter crates.

## Pros and Cons of the Options

### Option 1: Narrow port, two adapters

* Good, because small surface; semantics documented once.
* Bad, because features Celery gave for free (ETA, rate limits, chains) must be handled elsewhere (Workflow/Cron/consumer logic).

### Option 2: Celery-compatible wide API

* Good, because eases porting of the 32 task modules.
* Bad, because Cloudflare Queues cannot natively express ETA/chains/rate limits, so the adapter would emulate them, hiding costs.

### Option 3: DB-backed queue

* Good, because trivially portable and transactional with the outbox.
* Bad, because polling load on the system-of-record DB (Hyperdrive/D1) and no native DLQ/backoff; throughput ceiling.

### Option 4: CF Queues only

* Good, because least work.
* Bad, because violates ADR-0002.

## Update — interview round 20 (2026-10-18)

Ordering is guaranteed **only per work item** (per-item key where needed) (Q-0070.2).

## Update — review fixes, round 25 (2026-10-18)

* Rust sketch replaces the TypeScript one (ADR-0006); Cloudflare Queues limits verified in the independent review: **message ≤ 128 KB, batch ≤ 100 messages / 256 KB, delay 0–86,400 s, retention 60 s–14 d** — this **closes Q-0070.1**; `Envelope` payloads stay small (IDs, not content).
* The ordering mechanism for Q-0070.2 is named (per-`(workspace_id, entity_id)` key; single consumer per key). The earlier "no ordering guarantee" common subset is narrowed accordingly.
* Note: `workers-rs` marks Queues as beta (feature flag `queue`); confirmed by spike 0001.

## Update — spike results, round 26 (2026-10-18)

The `?Send` sketch above is corrected: ports return `Send` futures (ADR-0002).

Spike 0001 evidence (`[tested]` on local workerd and wrangler's Queues emulation only): the producer (`send_raw`) and the consumer (`#[event(queue)]`) are native in workers-rs 0.8.7 (the `queue` feature is marked beta upstream). **Send messages as JSON text:** passing a struct through serde-wasm-bindgen turns `serde_json::Value` objects into JS `Map`s that `JSON.stringify` renders as `{}` (derived from reading the code, not reproduced); text encoding avoids it. Consumer idempotency on `event_id` is verified by a redelivery test (the first "exactly once" test was vacuous and is now backed by a mutation control). Measured create-to-activity delivery was about 1.05 s with `max_batch_timeout = 1`, which is probably a batching floor (not varied, local only).

**Not tested:** `retry_with_options`, `max_retries`, the DLQ, the 128 KB message ceiling (Q-0070.1 stays as verified from the vendor docs), real delivery semantics, and ordering. Cloudflare Queues has no ordering primitive, so the per-key mechanism above (a single consumer per key through a DO, or a per-entity sequence check) still has to be built and tested.

## Open Questions

* Q-0073 — which tasks are Queue consumers vs Workflow vs Cron vs sync (mapping in ADR-0073).
* Q-0070.1 — **closed (round 25)**: 128 KB per message in the common subset.
* Q-0070.2 — **answered (round 20)**: ordering per work item only; mechanism above.

## More Information

* Plane: `apps/api/plane/celery.py`, `plane/bgtasks/*`; compose service `plane-mq` (rabbitmq 3.13.6).
* CF: Queues pull consumers <https://developers.cloudflare.com/queues/configuration/pull-consumers/>; wrangler config (queue consumer settings).
* Related: [[ADR-0002]], [[ADR-0046]], [[ADR-0071]], [[ADR-0073]], [[ADR-0121]].
