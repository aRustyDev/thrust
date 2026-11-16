//! The transactional outbox relay (ADR-0046): the after-commit fast path, the cron sweeper, and what the relay
//! hands the queue. Run against the memory database with a recording queue.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

/// Work items for the relay to publish events about, made as `/api/v1` makes them: `create_issue` writes the item and its
/// `work_item.created` outbox row in one transaction and relays after the commit. The subject here is the relay, not the use-case.
fn create(w: &World, n: usize) -> Vec<WorkItemRow> {
    let access = block_on(v1_issue_writer(&w.app, &w.principal(Actor::MemberA), &w.slug_a, w.project_a)).unwrap();
    (0..n)
        .map(|i| {
            let draft = IssueDraft { name: format!("item {i}"), ..IssueDraft::default() };
            block_on(create_issue(&w.app, &access, draft)).unwrap()
        })
        .collect()
}

fn depth(w: &World) -> u64 {
    block_on(outbox_depth(&w.app)).unwrap()
}

fn slow_path() -> (World, Arc<RecordingQueue>) {
    let queue = Arc::new(RecordingQueue::default());
    let cfg = CoreConfig { outbox_fastpath: false, ..CoreConfig::default() };
    let w = world_with(cfg, queue.clone(), Arc::new(NoBackground));
    (w, queue)
}

/// With the fast path off every committed row waits for the sweeper, which publishes each once and marks it.
#[test]
fn the_sweeper_publishes_every_waiting_row_exactly_once() {
    let (w, queue) = slow_path();
    let items = create(&w, 3);
    assert_eq!((depth(&w), queue.sent_len()), (3, 0), "nothing is published before the sweep");

    let rep = block_on(sweep_outbox(&w.app, 100)).unwrap();
    assert_eq!(rep, SweepReport { found: 3, published: 3, failed: 0 });
    assert_eq!((depth(&w), queue.sent_len()), (0, 3), "every row is published and marked dispatched");

    let sent: Vec<_> = queue.sent.lock().unwrap().iter().map(|(_, e, _)| e.entity_id).collect();
    let want: Vec<_> = items.iter().map(|i| i.item.id.as_uuid()).collect();
    assert_eq!(sent, want, "one event per created item, oldest first");

    let again = block_on(sweep_outbox(&w.app, 100)).unwrap();
    assert_eq!(again, SweepReport { found: 0, published: 0, failed: 0 }, "a dispatched row is not sent twice");
    assert_eq!(queue.sent_len(), 3);
}

/// A queue outage leaves every row waiting for the next sweep; nothing is lost and nothing is marked dispatched.
#[test]
fn a_failed_publish_leaves_the_row_for_the_next_sweep() {
    let (w, queue) = slow_path();
    create(&w, 2);
    queue.fail.store(true, Ordering::Relaxed);
    let rep = block_on(sweep_outbox(&w.app, 100)).unwrap();
    assert_eq!(rep, SweepReport { found: 2, published: 0, failed: 2 });
    assert_eq!(depth(&w), 2);

    queue.fail.store(false, Ordering::Relaxed);
    let rep = block_on(sweep_outbox(&w.app, 100)).unwrap();
    assert_eq!(rep, SweepReport { found: 2, published: 2, failed: 0 });
    assert_eq!(depth(&w), 0);
}

/// The relay asks the queue to deduplicate on the event id and to keep each entity's events in order.
#[test]
fn the_relay_sends_dedupe_and_ordering_keys() {
    let (w, queue) = slow_path();
    let item = create(&w, 1).remove(0);
    block_on(sweep_outbox(&w.app, 100)).unwrap();
    let sent = queue.sent.lock().unwrap();
    let (topic, env, opts) = &sent[0];
    assert_eq!(topic, EVENTS_TOPIC);
    assert_eq!(env.event_type, "work_item.created");
    assert_eq!(opts.idempotency_key.as_deref(), Some(env.event_id.to_string().as_str()));
    assert_eq!(opts.ordering_key.as_deref(), Some(format!("{}:{}", w.ws_a, item.item.id).as_str()));
}

/// With the fast path on, the row is published by the after-commit task, not left for the sweeper.
#[test]
fn the_fast_path_publishes_after_commit() {
    let queue = Arc::new(RecordingQueue::default());
    let background = Arc::new(DeferredBackground::default());
    let cfg = CoreConfig { outbox_fastpath: true, ..CoreConfig::default() };
    let w = world_with(cfg, queue.clone(), background.clone());

    create(&w, 1);
    assert_eq!((depth(&w), queue.sent_len()), (1, 0), "committed, not yet relayed");
    assert_eq!(background.run_all(), 1, "the relay was scheduled once");
    assert_eq!((depth(&w), queue.sent_len()), (0, 1));
    assert_eq!(block_on(sweep_outbox(&w.app, 100)).unwrap().found, 0, "nothing left for the sweeper");
}

/// The fast path swallows a publish failure on purpose: the row stays, and the sweeper delivers it later.
#[test]
fn a_failed_fast_path_falls_back_to_the_sweeper() {
    let queue = Arc::new(RecordingQueue::default());
    let background = Arc::new(DeferredBackground::default());
    let cfg = CoreConfig { outbox_fastpath: true, ..CoreConfig::default() };
    let w = world_with(cfg, queue.clone(), background.clone());

    queue.fail.store(true, Ordering::Relaxed);
    create(&w, 1);
    background.run_all();
    assert_eq!((depth(&w), queue.sent_len()), (1, 0));

    queue.fail.store(false, Ordering::Relaxed);
    assert_eq!(block_on(sweep_outbox(&w.app, 100)).unwrap(), SweepReport { found: 1, published: 1, failed: 0 });
}

/// The consumer entrypoint every queue adapter calls: it projects the event into activity, and reports a bad event
/// as an error instead of acknowledging it.
#[test]
fn the_activity_consumer_records_events_and_rejects_bad_ones() {
    let w = world();
    let consumer = ActivityConsumer(w.app.clone());
    let item = create(&w, 1).remove(0);
    let mut env = block_on(w.app.ports.db.undispatched_outbox(&SystemScope::mint(), 10)).unwrap().remove(0).envelope;

    block_on(consumer.handle(&env)).unwrap();
    let rows = block_on(list_activity(&w.app, &w.principal(Actor::GuestA), &w.slug_a, w.project_a, Some(item.item.id)))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].verb, "created");

    env.payload = serde_json::json!({});
    assert!(block_on(consumer.handle(&env)).is_err(), "an event without a project id is not acknowledged");
}
