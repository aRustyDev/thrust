//! What a queue adapter that receives a message as text does with one delivery of it (ADR-0070, FR-21, issue #53). The Worker's consumer
//! is such an adapter, and it is the only one: a message reaches it as a string, which may not be an event at all, so decoding it is part
//! of the policy and not something the consumer can be asked.
//!
//! These are functions of what arrived and what the consumer answered, with no platform in them, so the decisions are tested natively
//! and the Worker is left to read a message, call one of them and act on its answer.

use std::time::Duration;

use thrust_contracts::{DeadLetterMessage, Envelope};

use crate::{disposition, ConsumerError, Disposition, EventConsumer, NewDeadLetter};

/// What to do with one delivery of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// Done: acknowledge it.
    Ack,
    /// Deliver it again after `delay`; `error` is why, for the adapter's log (the message itself is not kept anywhere yet).
    Retry { delay: Duration, error: String },
    /// Set it aside: put this where the adapter keeps dead letters, and then acknowledge the message. If putting it there fails, the
    /// message is not acknowledged, so that it is delivered again.
    SetAside(NewDeadLetter),
}

/// One delivery of a message of the events topic. `body` is the text that arrived, `source_id` the queue's own id for the message,
/// `deliveries` how many times it has been delivered, this one included (`1` the first time).
///
/// A body that is not an event is wrong for good, so it is set aside at once, with its text exactly as it came and no event id, and the
/// consumer is not asked about it. An event is handed to the consumer and its answer goes through [`disposition`].
pub async fn settle(
    consumer: &dyn EventConsumer,
    topic: &str,
    source_id: &str,
    body: &str,
    deliveries: u32,
) -> Settled {
    let (event_id, outcome) = match serde_json::from_str::<Envelope>(body) {
        Ok(env) => (Some(env.event_id), consumer.handle(&env).await),
        Err(e) => (None, Err(ConsumerError::Permanent(format!("the message is not an event: {e}")))),
    };
    let error = outcome.as_ref().err().map(ToString::to_string).unwrap_or_default();
    match disposition(&outcome, deliveries) {
        Disposition::Ack => Settled::Ack,
        Disposition::Retry(delay) => Settled::Retry { delay, error },
        Disposition::DeadLetter => Settled::SetAside(NewDeadLetter {
            source_id: Some(source_id.to_owned()),
            topic: topic.to_owned(),
            event_id,
            body: body.to_owned(),
            error,
            attempts: Some(deliveries),
        }),
    }
}

/// One message from the events dead-letter queue, which holds two kinds. One is what [`settle`] set aside, sent there as a
/// [`DeadLetterMessage`] that says what it was and why: it becomes the row it describes, under the id the queue gave the message
/// that was set aside. The other is a message that the queue itself moved there because its retries ran out, whose body is the original
/// and which knows nothing of why: it becomes a row under its own id with no count of deliveries (the queue starts that count again
/// when it moves a message), and an event id if the body is an event.
///
/// `Err` when the write failed; the adapter delivers the message again, which is safe because a row is written once per id.
pub async fn settle_dead_letter(
    consumer: &dyn EventConsumer,
    topic: &str,
    source_id: &str,
    body: &str,
) -> Result<(), ConsumerError> {
    let letter = match DeadLetterMessage::parse(body) {
        Some(m) => NewDeadLetter {
            source_id: Some(m.source_id),
            topic: m.topic,
            event_id: m.event_id,
            body: m.body,
            error: m.error,
            attempts: Some(m.attempts),
        },
        None => NewDeadLetter {
            source_id: Some(source_id.to_owned()),
            topic: topic.to_owned(),
            event_id: serde_json::from_str::<Envelope>(body).ok().map(|e| e.event_id),
            body: body.to_owned(),
            error: "set aside by the queue after its retries ran out".to_owned(),
            attempts: None,
        },
    };
    consumer.dead_letter(&letter).await
}
