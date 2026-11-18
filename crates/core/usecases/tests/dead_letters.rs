//! Setting a message aside (ADR-0070, issue #53): `record_dead_letter` and `dead_letters`, against the memory adapter. What the
//! consumer adds to what a queue adapter hands it (the time, an id when the queue has none, text the database can keep), and that a
//! message set aside twice is one row.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

fn letter(source_id: Option<&str>, body: &str) -> NewDeadLetter {
    NewDeadLetter {
        source_id: source_id.map(str::to_owned),
        topic: "events".into(),
        event_id: None,
        body: body.into(),
        error: "permanent: invalid: event without project_id".into(),
        attempts: Some(1),
    }
}

fn arbitrary_letter() -> impl Strategy<Value = NewDeadLetter> {
    // any text at all, which is what a producer can send, NULs included: they are the one thing a row cannot keep
    let text = prop_oneof![3 => any::<String>(), 1 => "[a\u{0}b\u{FFFD}]{0,6}"];
    (
        prop::option::of("[a-z0-9-]{1,12}"),
        text.clone(),
        prop::option::of(any::<u128>()),
        text.clone(),
        text,
        prop::option::of(1..=u32::MAX),
    )
        .prop_map(|(source_id, topic, event, body, error, attempts)| NewDeadLetter {
            source_id,
            topic,
            event_id: event.map(|n| EventId(Uuid::from_u128(n))),
            body,
            error,
            attempts,
        })
}

/// What a text must come back as: the same characters, with a NUL (which Postgres cannot store) in the place of each NUL.
fn kept(text: &str) -> String {
    text.chars().map(|c| if c == '\0' { '\u{FFFD}' } else { c }).collect()
}

proptest! {
    #![proptest_config(common::proptest_config(64))]

    /// Whatever a queue adapter hands the consumer reads back as it was, except that a NUL is replaced, and the consumer adds a time that
    /// is when it wrote it. One letter, one row.
    #[test]
    fn prop_a_recorded_dead_letter_reads_back_as_given(given in arbitrary_letter()) {
        let w = world();
        let before = w.app.ports.clock.now();
        prop_assert!(block_on(record_dead_letter(&w.app, &given)).unwrap(), "the first write inserts");
        let after = w.app.ports.clock.now();

        let rows = block_on(dead_letters(&w.app, 10)).unwrap();
        prop_assert_eq!(rows.len(), 1);
        let row = &rows[0];
        prop_assert_eq!(&row.topic, &kept(&given.topic));
        prop_assert_eq!(&row.body, &kept(&given.body));
        prop_assert_eq!(&row.error, &kept(&given.error));
        prop_assert_eq!(row.event_id, given.event_id);
        prop_assert_eq!(row.attempts, given.attempts);
        prop_assert!(row.received_at >= before && row.received_at <= after, "{:?} not in [{:?}, {:?}]", row.received_at, before, after);
        match &given.source_id {
            Some(id) => prop_assert_eq!(&row.id, id, "the queue's id is the row's id"),
            None => prop_assert!(!row.id.is_empty(), "a message the queue gave no id gets one"),
        }
    }
}

/// The queue's id is what makes a delivery that comes again land on the row it made the first time, and the first answer stands.
#[test]
fn a_message_set_aside_twice_is_one_row_and_the_first_stands() {
    let w = world();
    assert!(block_on(record_dead_letter(&w.app, &letter(Some("msg-1"), "first"))).unwrap());
    assert!(
        !block_on(record_dead_letter(&w.app, &letter(Some("msg-1"), "second"))).unwrap(),
        "the redelivery inserts nothing"
    );
    // the control: another message is another row, so the above is the id and not "only one row ever"
    assert!(block_on(record_dead_letter(&w.app, &letter(Some("msg-2"), "other"))).unwrap());

    let rows = block_on(dead_letters(&w.app, 10)).unwrap();
    let bodies: Vec<_> = rows.iter().map(|r| (r.id.as_str(), r.body.as_str())).collect();
    assert_eq!(rows.len(), 2, "{bodies:?}");
    assert!(bodies.contains(&("msg-1", "first")), "{bodies:?}");
    assert!(bodies.contains(&("msg-2", "other")), "{bodies:?}");
}

/// A queue with no id for a message (the in-process one) still must not lose two messages to one row.
#[test]
fn two_messages_without_ids_are_two_rows() {
    let w = world();
    assert!(block_on(record_dead_letter(&w.app, &letter(None, "one"))).unwrap());
    assert!(
        block_on(record_dead_letter(&w.app, &letter(None, "one"))).unwrap(),
        "the same text is not the same message"
    );
    let rows = block_on(dead_letters(&w.app, 10)).unwrap();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].id, rows[1].id);
}
