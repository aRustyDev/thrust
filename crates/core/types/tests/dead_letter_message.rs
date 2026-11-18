//! The wire type of the events dead-letter queue (ADR-0070, issue #53): what the Worker writes there when it gives up on a message, and
//! what it must never take for one. The queue also holds messages that Cloudflare moved there itself, whose bodies are whatever a
//! producer sent, so `DeadLetterMessage::parse` is a parser of outside text and is fuzzed (`src/fuzz_harness.rs`).
//!
//! The properties below are what the fuzz target also checks, and they run on every `cargo nextest run`.

mod common;

use proptest::prelude::*;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;

fn arbitrary_message() -> impl Strategy<Value = DeadLetterMessage> {
    (any::<String>(), any::<String>(), prop::option::of(any::<u128>()), any::<String>(), any::<String>(), any::<u32>())
        .prop_map(|(source_id, topic, event, body, error, attempts)| {
            DeadLetterMessage::new(source_id, topic, event.map(|n| EventId(Uuid::from_u128(n))), body, error, attempts)
        })
}

/// Every envelope there can be, as far as its fields go: a producer's own message, which is what lands on the queue most.
fn arbitrary_envelope() -> impl Strategy<Value = Envelope> {
    (
        any::<u128>(),
        any::<u32>(),
        any::<u128>(),
        any::<String>(),
        any::<String>(),
        any::<u128>(),
        prop::option::of(any::<u128>()),
        0..4_000_000_000i64,
        any::<String>(),
        prop::option::of(any::<String>()),
    )
        .prop_map(|(event, version, workspace, event_type, entity_type, entity, actor, at, note, trace_id)| {
            Envelope {
                event_id: EventId(Uuid::from_u128(event)),
                schema_version: version,
                workspace_id: WorkspaceId(Uuid::from_u128(workspace)),
                event_type,
                entity_type,
                entity_id: Uuid::from_u128(entity),
                actor_id: actor.map(|n| UserId(Uuid::from_u128(n))),
                occurred_at: DateTime::from_timestamp(at, 0).unwrap(),
                payload: serde_json::json!({ "note": note }),
                trace_id,
            }
        })
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    /// What the Worker writes, the Worker reads back: every field, whatever the text in it.
    #[test]
    fn prop_a_dead_letter_message_reads_back_as_it_was_written(m in arbitrary_message()) {
        let text = m.to_text().unwrap();
        prop_assert_eq!(DeadLetterMessage::parse(&text), Some(m));
    }

    /// The marker is what tells our messages from a producer's: the same JSON under another marker is somebody else's.
    #[test]
    fn prop_a_message_under_another_marker_is_not_taken(m in arbitrary_message(), kind in any::<String>()) {
        prop_assume!(kind != DEAD_LETTER_MESSAGE_KIND);
        let other = DeadLetterMessage { kind, ..m };
        prop_assert_eq!(DeadLetterMessage::parse(&other.to_text().unwrap()), None);
    }

    /// A producer's message, the kind that lands on the queue most, is never taken for one of ours.
    #[test]
    fn prop_an_envelope_is_never_taken_for_a_dead_letter_message(env in arbitrary_envelope()) {
        prop_assert_eq!(DeadLetterMessage::parse(&serde_json::to_string(&env).unwrap()), None);
    }

    /// Any text at all gets an answer and not a panic, and the only text that answers `Some` carries the marker.
    #[test]
    fn prop_any_text_is_answered_and_only_the_marked_shape_is_taken(text in any::<String>()) {
        if let Some(m) = DeadLetterMessage::parse(&text) {
            prop_assert_eq!(m.kind, DEAD_LETTER_MESSAGE_KIND);
        }
    }

    /// The shape is exact: a field that is not ours turns a message into somebody else's, even with our marker.
    #[test]
    fn prop_a_message_with_an_extra_field_is_not_taken(m in arbitrary_message(), key in "[a-z]{1,8}") {
        prop_assume!(!["kind", "source_id", "topic", "event_id", "body", "error", "attempts"].contains(&key.as_str()));
        let mut value: serde_json::Value = serde_json::from_str(&m.to_text().unwrap()).unwrap();
        value.as_object_mut().unwrap().insert(key, serde_json::json!(true));
        prop_assert_eq!(DeadLetterMessage::parse(&value.to_string()), None);
    }
}
