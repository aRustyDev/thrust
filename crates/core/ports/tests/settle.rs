//! What a queue adapter that receives text does with one delivery of a message (ADR-0070, FR-21, issue #53): `settle` for the events
//! queue and `settle_dead_letter` for its dead-letter queue. The Worker's consumer is a thin shell around them, so what it does with
//! every kind of message is decided, and tested, here.
//!
//! The rules, written out and not read from the crate:
//!
//! * a body that is not an event is set aside at once, exactly as it came, with no event id, and the consumer is never asked about it;
//! * an event the consumer accepts is done; a permanent error sets it aside at once; a transient error is delivered again after 5, 10
//!   and 20 seconds for deliveries 1, 2 and 3, and sets it aside on the 4th;
//! * a message set aside carries the queue's id for it, the topic, the event's id, the text as it came, why, and how many deliveries;
//! * on the dead-letter queue a message of ours becomes the row it describes, and any other becomes a row under the queue's own id with
//!   no count of deliveries.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;

/// A consumer that gives one answer to every event and one to every write, and keeps what it was given.
struct Fixed {
    answer: Result<(), ConsumerError>,
    write: Result<(), ConsumerError>,
    handled: AtomicUsize,
    letters: Mutex<Vec<NewDeadLetter>>,
}

impl Fixed {
    fn new(answer: Result<(), ConsumerError>) -> Self {
        Self { answer, write: Ok(()), handled: AtomicUsize::new(0), letters: Mutex::new(Vec::new()) }
    }
}

#[async_trait]
impl EventConsumer for Fixed {
    async fn handle(&self, _env: &Envelope) -> Result<(), ConsumerError> {
        self.handled.fetch_add(1, Ordering::Relaxed);
        self.answer.clone()
    }

    async fn dead_letter(&self, letter: &NewDeadLetter) -> Result<(), ConsumerError> {
        self.letters.lock().unwrap().push(letter.clone());
        self.write.clone()
    }
}

fn envelope(n: u128) -> Envelope {
    Envelope {
        event_id: EventId(Uuid::from_u128(n)),
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

#[derive(Debug, Clone, Copy)]
enum Answer {
    Done,
    Permanent,
    Transient,
}

#[rstest]
fn every_answer_and_delivery_count_gets_the_settlement_of_the_table(
    #[values(Answer::Done, Answer::Permanent, Answer::Transient)] answer: Answer,
    #[values(1, 2, 3, 4, 5)] deliveries: u32,
) {
    let (result, error) = match answer {
        Answer::Done => (Ok(()), ""),
        Answer::Permanent => (Err(ConsumerError::Permanent("p".into())), "permanent: p"),
        Answer::Transient => (Err(ConsumerError::Transient("t".into())), "transient: t"),
    };
    let consumer = Fixed::new(result);
    let env = envelope(0xE1);
    let body = serde_json::to_string(&env).unwrap();

    let got = block_on(settle(&consumer, "events", "msg-7", &body, deliveries));

    let set_aside = Settled::SetAside(NewDeadLetter {
        source_id: Some("msg-7".into()),
        topic: "events".into(),
        event_id: Some(env.event_id),
        body: body.clone(),
        error: error.into(),
        attempts: Some(deliveries),
    });
    let want = match (answer, deliveries) {
        (Answer::Done, _) => Settled::Ack,
        (Answer::Permanent, _) => set_aside,
        (Answer::Transient, 1) => Settled::Retry { delay: Duration::from_secs(5), error: error.into() },
        (Answer::Transient, 2) => Settled::Retry { delay: Duration::from_secs(10), error: error.into() },
        (Answer::Transient, 3) => Settled::Retry { delay: Duration::from_secs(20), error: error.into() },
        (Answer::Transient, _) => set_aside,
    };
    assert_eq!(got, want, "{answer:?} on delivery {deliveries}");
    assert_eq!(consumer.handled.load(Ordering::Relaxed), 1, "the consumer is asked once about an event");
    assert!(consumer.letters.lock().unwrap().is_empty(), "settle never writes: the adapter does, with what it answers");
}

/// Texts that are not events: any text, text built from the characters that go wrong (a NUL, whitespace, the ends of JSON), text with
/// whitespace at either end (which `any::<String>()` all but never has), and JSON of other shapes (arrays, objects with other keys).
fn not_an_event() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => any::<String>(),
        2 => "[ \t\n\r\u{0}{}\\[\\]\"\\\\a:,]{0,16}",
        2 => ("[ \t\n\r]{0,3}", any::<String>(), "[ \t\n\r]{0,3}").prop_map(|(head, text, tail)| format!("{head}{text}{tail}")),
        1 => prop::collection::vec(any::<String>(), 0..3).prop_map(|v| serde_json::to_string(&v).unwrap()),
        1 => prop::collection::hash_map("[a-z]{1,5}", any::<i32>(), 0..4).prop_map(|m| serde_json::to_string(&m).unwrap()),
    ]
    .prop_filter("an event is not what this is about", |s| serde_json::from_str::<Envelope>(s).is_err())
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    /// Whatever else a producer sends, it is set aside whole and at once: its text as it came (a NUL, a lone brace, 100 KB of anything),
    /// nothing of an event in the row, and the consumer never asked. The first delivery already; the count is the queue's.
    #[test]
    fn prop_a_body_that_is_not_an_event_is_set_aside_at_once_and_untouched(body in not_an_event(), deliveries in 1..=u32::MAX) {
        let consumer = Fixed::new(Ok(()));
        let got = block_on(settle(&consumer, "events", "msg-1", &body, deliveries));
        match got {
            Settled::SetAside(letter) => {
                prop_assert_eq!(&letter.body, &body);
                prop_assert_eq!(letter.event_id, None);
                prop_assert_eq!(letter.attempts, Some(deliveries));
                prop_assert_eq!(letter.source_id.as_deref(), Some("msg-1"));
                prop_assert_eq!(&letter.topic, "events");
                prop_assert!(letter.error.starts_with("permanent: "), "{}", letter.error);
            }
            other => prop_assert!(false, "a body that is not an event was not set aside: {:?}", other),
        }
        prop_assert_eq!(consumer.handled.load(Ordering::Relaxed), 0, "the consumer is not asked about what is not an event");
    }

    /// The dead-letter queue holds what the Worker set aside, as `DeadLetterMessage`s: each becomes the row it says it is, in its own
    /// words and not the queue's (the id, the topic and the count are those of the message that was set aside).
    #[test]
    fn prop_a_message_of_ours_becomes_the_row_it_describes(
        source_id in any::<String>(), topic in any::<String>(), event in prop::option::of(any::<u128>()),
        body in any::<String>(), error in any::<String>(), attempts in any::<u32>(),
    ) {
        let m = DeadLetterMessage::new(source_id.clone(), topic.clone(), event.map(|n| EventId(Uuid::from_u128(n))), body.clone(), error.clone(), attempts);
        let consumer = Fixed::new(Ok(()));
        let result = block_on(settle_dead_letter(&consumer, "events", "the-dlq-own-id", &m.to_text().unwrap()));
        prop_assert_eq!(result, Ok(()));
        let letters = consumer.letters.lock().unwrap();
        prop_assert_eq!(letters.len(), 1);
        prop_assert_eq!(
            &letters[0],
            &NewDeadLetter {
                source_id: Some(source_id),
                topic,
                event_id: event.map(|n| EventId(Uuid::from_u128(n))),
                body,
                error,
                attempts: Some(attempts),
            }
        );
    }

    /// What the queue itself moved there is the original message: any text. It becomes a row under the queue's own id for it, whole, with
    /// no count of deliveries (the queue starts the count again) and with the event's id if it is an event.
    #[test]
    fn prop_a_message_the_queue_moved_becomes_a_row_under_its_own_id(body in prop_oneof![not_an_event(), Just(serde_json::to_string(&envelope(0xE1)).unwrap())]) {
        let consumer = Fixed::new(Ok(()));
        let result = block_on(settle_dead_letter(&consumer, "events", "the-dlq-own-id", &body));
        prop_assert_eq!(result, Ok(()));
        let letters = consumer.letters.lock().unwrap();
        prop_assert_eq!(letters.len(), 1);
        let letter = &letters[0];
        prop_assert_eq!(letter.source_id.as_deref(), Some("the-dlq-own-id"));
        prop_assert_eq!(&letter.topic, "events");
        prop_assert_eq!(&letter.body, &body);
        prop_assert_eq!(letter.attempts, None);
        let is_event = serde_json::from_str::<Envelope>(&body).is_ok();
        prop_assert_eq!(letter.event_id.is_some(), is_event);
        prop_assert!(letter.error.contains("retries ran out"), "{}", letter.error);
    }
}

/// A write that fails is reported, so the adapter delivers the message again (a row is written once per id, so that is safe).
#[rstest]
fn a_failed_write_is_an_error_for_both_kinds_of_message(
    #[values(
        "an original message of someone's",
        r#"{"kind":"thrust.dead-letter.v1","source_id":"s","topic":"events","event_id":null,"body":"b","error":"e","attempts":1}"#
    )]
    body: &str,
) {
    let mut consumer = Fixed::new(Ok(()));
    consumer.write = Err(ConsumerError::Transient("the database is down".into()));
    let result = block_on(settle_dead_letter(&consumer, "events", "id", body));
    assert_eq!(result, Err(ConsumerError::Transient("the database is down".into())));
    assert_eq!(consumer.letters.lock().unwrap().len(), 1, "the write was tried");
}
