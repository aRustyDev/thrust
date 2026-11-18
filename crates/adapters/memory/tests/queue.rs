//! The in-process queue (ADR-0070, FR-21, issue #53): what it does with a message after the consumer answered, and with the dead letter
//! of a message it gave up on.
//!
//! The policy is written out below and not read from `thrust_ports`, so the test is a check of the queue against the rule and not of the
//! queue against itself: a message is delivered four times at most, waiting 5, 10 and 20 seconds between them; a permanent error is
//! set aside after its first delivery; a done message is never set aside; the write of a dead letter gets the same four tries and the
//! same waits, and a message whose write fails four times is lost (and counted).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::proptest_config;
use futures_executor::block_on;
use proptest::prelude::*;
use thrust_adapters_memory::{ConsumerSlot, MemoryQueue, Sleep};
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;

/// The policy: at most four deliveries, and the seconds to wait after the first, second and third.
const MAX: usize = 4;
const WAITS: [u64; 3] = [5, 10, 20];

#[derive(Debug, Clone, Copy)]
enum Answer {
    Done,
    Permanent,
    Transient,
}

impl Answer {
    fn result(self) -> Result<(), ConsumerError> {
        match self {
            Answer::Done => Ok(()),
            Answer::Permanent => Err(ConsumerError::Permanent("p".into())),
            Answer::Transient => Err(ConsumerError::Transient("t".into())),
        }
    }
}

/// What must have happened to a message the consumer answered `answers` to (a transient error when the list runs out), when the first
/// `failing_writes` writes of its dead letter fail.
#[derive(Debug, PartialEq)]
struct Expected {
    handled: usize,
    /// The deliveries the dead letter records, when there is one.
    letter: Option<usize>,
    /// The error text the dead letter records.
    error: Option<&'static str>,
    writes: usize,
    lost: bool,
    waits: Vec<u64>,
}

fn expect(answers: &[Answer], failing_writes: usize) -> Expected {
    let mut waits = Vec::new();
    let mut delivery = 0;
    let error = loop {
        delivery += 1;
        match answers.get(delivery - 1).copied().unwrap_or(Answer::Transient) {
            Answer::Done => {
                return Expected { handled: delivery, letter: None, error: None, writes: 0, lost: false, waits };
            }
            Answer::Permanent => break "permanent: p",
            Answer::Transient if delivery == MAX => break "transient: t",
            Answer::Transient => waits.push(WAITS[delivery - 1]),
        }
    };
    let mut write = 0;
    loop {
        write += 1;
        if write > failing_writes {
            return Expected {
                handled: delivery,
                letter: Some(delivery),
                error: Some(error),
                writes: write,
                lost: false,
                waits,
            };
        }
        if write == MAX {
            return Expected { handled: delivery, letter: None, error: None, writes: write, lost: true, waits };
        }
        waits.push(WAITS[write - 1]);
    }
}

/// A consumer that answers as scripted and keeps what it was given.
struct Scripted {
    answers: Mutex<VecDeque<Result<(), ConsumerError>>>,
    failing_writes: AtomicUsize,
    handled: Mutex<Vec<Envelope>>,
    writes: AtomicUsize,
    letters: Mutex<Vec<NewDeadLetter>>,
}

#[async_trait]
impl EventConsumer for Scripted {
    async fn handle(&self, env: &Envelope) -> Result<(), ConsumerError> {
        self.handled.lock().unwrap().push(env.clone());
        self.answers.lock().unwrap().pop_front().unwrap_or_else(|| Err(ConsumerError::Transient("t".into())))
    }

    async fn dead_letter(&self, letter: &NewDeadLetter) -> Result<(), ConsumerError> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        let failing = self.failing_writes.load(Ordering::Relaxed);
        if failing > 0 {
            self.failing_writes.store(failing - 1, Ordering::Relaxed);
            return Err(ConsumerError::Transient("the database is down".into()));
        }
        self.letters.lock().unwrap().push(letter.clone());
        Ok(())
    }
}

/// Keeps what it is given until the test runs it: `send` is itself driven by an executor, which cannot be entered again from inside
/// `spawn`. The memory adapter never yields, so a delivery is over when `run_all` returns.
#[derive(Default)]
struct Deferred(Mutex<Vec<BoxFuture<()>>>);

impl Deferred {
    fn run_all(&self) {
        let tasks: Vec<_> = self.0.lock().unwrap().drain(..).collect();
        for task in tasks {
            block_on(task);
        }
    }
}

impl Background for Deferred {
    fn spawn(&self, fut: BoxFuture<()>) {
        self.0.lock().unwrap().push(fut);
    }
}

fn envelope() -> Envelope {
    Envelope {
        event_id: EventId(Uuid::from_u128(0xE1)),
        schema_version: 1,
        workspace_id: WorkspaceId(Uuid::from_u128(0xA1)),
        event_type: "work_item.created".into(),
        entity_type: "work_item".into(),
        entity_id: Uuid::from_u128(0xE2),
        actor_id: None,
        occurred_at: DateTime::from_timestamp(1_760_000_000, 0).unwrap(),
        payload: serde_json::json!({ "project_id": ProjectId(Uuid::from_u128(0xB1)) }),
        trace_id: None,
    }
}

fn answers() -> impl Strategy<Value = Vec<Answer>> {
    let answer = prop_oneof![1 => Just(Answer::Done), 1 => Just(Answer::Permanent), 4 => Just(Answer::Transient)];
    prop::collection::vec(answer, 0..6)
}

proptest! {
    #![proptest_config(proptest_config(256))]

    #[test]
    fn model_memory_queue_follows_the_delivery_policy(answers in answers(), failing_writes in 0..6usize, waits in any::<bool>()) {
        let consumer = Arc::new(Scripted {
            answers: Mutex::new(answers.iter().map(|a| a.result()).collect()),
            failing_writes: AtomicUsize::new(failing_writes),
            handled: Mutex::new(Vec::new()),
            writes: AtomicUsize::new(0),
            letters: Mutex::new(Vec::new()),
        });
        let slot: ConsumerSlot = Default::default();
        slot.set(consumer.clone()).ok();
        let slept = Arc::new(Mutex::new(Vec::<Duration>::new()));
        let background = Arc::new(Deferred::default());
        let mut queue = MemoryQueue::new(background.clone(), slot);
        if waits {
            let log = slept.clone();
            let sleep: Sleep = Arc::new(move |d| {
                log.lock().unwrap().push(d);
                Box::pin(async {})
            });
            queue = queue.with_sleep(sleep);
        }

        let env = envelope();
        block_on(queue.send("events", &env, SendOpts::default())).unwrap();
        prop_assert!(consumer.handled.lock().unwrap().is_empty(), "nothing is delivered before the background runs");
        background.run_all();

        let want = expect(&answers, failing_writes);
        let handled = consumer.handled.lock().unwrap();
        prop_assert_eq!(handled.len(), want.handled, "deliveries of {:?} with {} failing writes", answers, failing_writes);
        prop_assert!(handled.iter().all(|e| *e == env), "every delivery is of the message that was sent");
        prop_assert_eq!(consumer.writes.load(Ordering::Relaxed), want.writes, "writes of the dead letter");
        prop_assert_eq!(queue.lost_count(), usize::from(want.lost), "lost");

        let letters = consumer.letters.lock().unwrap();
        match want.letter {
            None => prop_assert!(letters.is_empty(), "a letter nobody should have written: {:?}", letters),
            Some(attempts) => {
                prop_assert_eq!(letters.len(), 1);
                let letter = &letters[0];
                prop_assert_eq!(letter.attempts, Some(u32::try_from(attempts).unwrap()));
                prop_assert_eq!(&letter.topic, "events");
                prop_assert_eq!(letter.event_id, Some(env.event_id));
                prop_assert_eq!(letter.source_id.as_deref(), None, "an in-process queue has no id for a message");
                prop_assert_eq!(Some(letter.error.as_str()), want.error);
                let body: Envelope = serde_json::from_str(&letter.body).unwrap();
                prop_assert_eq!(body, env, "the body is the message");
            }
        }

        // the waits are those of the policy, in order, when the queue can wait; and there are none when it cannot
        let slept: Vec<u64> = slept.lock().unwrap().iter().map(Duration::as_secs).collect();
        prop_assert_eq!(slept, if waits { want.waits } else { Vec::new() }, "waits");
    }
}
