//! What a failure of the queue consumer means for the message (ADR-0070, FR-21, issue #53): whether delivering it again can help.
//! `consumer_error` names a kind for every `CoreError`, and `ActivityConsumer::handle` answers with it, against the memory adapter.
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
use rstest::rstest;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Permanent,
    Transient,
}

fn kind(e: &ConsumerError) -> Kind {
    match e {
        ConsumerError::Permanent(_) => Kind::Permanent,
        ConsumerError::Transient(_) => Kind::Transient,
    }
}

/// Adding a variant to `CoreError` stops this from compiling until the table below has a case for it, as `consumer_error` itself does.
#[allow(dead_code)]
fn every_variant_has_a_case(e: CoreError) {
    match e {
        CoreError::Unauthenticated
        | CoreError::Forbidden
        | CoreError::Csrf
        | CoreError::InvalidApiKey
        | CoreError::NotFound
        | CoreError::Conflict(_)
        | CoreError::RateLimited { .. }
        | CoreError::Invalid(_)
        | CoreError::TooLarge
        | CoreError::Upstream(_)
        | CoreError::Internal(_) => {}
    }
}

// The table: an error about the message is permanent; an error about something the consumer needed is transient.
#[rstest]
#[case::invalid(CoreError::Invalid("event without project_id".into()), Kind::Permanent)]
#[case::not_found(CoreError::NotFound, Kind::Permanent)]
#[case::unauthenticated(CoreError::Unauthenticated, Kind::Permanent)]
#[case::forbidden(CoreError::Forbidden, Kind::Permanent)]
#[case::csrf(CoreError::Csrf, Kind::Permanent)]
#[case::invalid_api_key(CoreError::InvalidApiKey, Kind::Permanent)]
#[case::too_large(CoreError::TooLarge, Kind::Permanent)]
#[case::internal(CoreError::Internal("db: connection refused".into()), Kind::Transient)]
#[case::upstream(CoreError::Upstream("the provider answered 502".into()), Kind::Transient)]
#[case::rate_limited(CoreError::RateLimited { retry_after_secs: 3 }, Kind::Transient)]
#[case::conflict(CoreError::Conflict("duplicate key".into()), Kind::Transient)]
fn every_core_error_has_the_kind_of_the_table(#[case] error: CoreError, #[case] want: Kind) {
    let text = error.to_string();
    let got = consumer_error(error);
    assert_eq!(kind(&got), want, "{got:?}");
    assert!(got.to_string().contains(&text), "the kind keeps the reason: {got} does not carry {text}");
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    /// The kind belongs to the variant and never to what the message says: whatever text an error carries, and whatever it happens to
    /// mention ("not found", "timeout", "invalid"), it is the same kind.
    #[test]
    fn prop_the_kind_depends_on_the_variant_and_never_on_its_text(text in ".*", secs in any::<u64>()) {
        prop_assert_eq!(kind(&consumer_error(CoreError::Invalid(text.clone()))), Kind::Permanent);
        for e in [
            CoreError::Internal(text.clone()),
            CoreError::Upstream(text.clone()),
            CoreError::Conflict(text.clone()),
            CoreError::RateLimited { retry_after_secs: secs },
        ] {
            prop_assert_eq!(kind(&consumer_error(e)), Kind::Transient);
        }
    }
}

/// An event of a work item, as the outbox writes one, for the tests below to damage. `n` makes its id.
fn event(w: &World, n: u128, entity: Uuid, payload: serde_json::Value) -> Envelope {
    Envelope {
        event_id: EventId(Uuid::from_u128(n)),
        schema_version: 1,
        workspace_id: w.ws_a,
        event_type: "work_item.updated".into(),
        entity_type: "work_item".into(),
        entity_id: entity,
        actor_id: Some(w.principal(Actor::MemberA).user_id),
        occurred_at: w.app.ports.clock.now(),
        payload,
        trace_id: None,
    }
}

fn history() -> serde_json::Value {
    serde_json::json!([{
        "verb": "updated", "field": "name", "old_value": "before", "new_value": "after", "comment": "",
        "old_identifier": null, "new_identifier": null
    }])
}

fn existing_work_item(w: &World) -> Uuid {
    let access = block_on(v1_issue_writer(&w.app, &w.principal(Actor::MemberA), &w.slug_a, w.project_a)).unwrap();
    let draft = IssueDraft { name: "an item".into(), ..IssueDraft::default() };
    block_on(create_issue(&w.app, &access, draft)).unwrap().item.id.as_uuid()
}

/// FR-21 and research 0004 (S2-9) name this event: it has no `project_id`, so no delivery of it can ever work, and it must not be
/// retried. This is what made Stage 2 deliver it four times before the queue gave up.
#[test]
fn an_event_without_a_project_id_is_permanent() {
    let w = world();
    let consumer = ActivityConsumer(w.app.clone());
    let got = block_on(consumer.handle(&event(&w, 0xE1, Uuid::from_u128(0xE2), serde_json::json!({})))).unwrap_err();
    assert_eq!(kind(&got), Kind::Permanent, "{got:?}");
    assert!(got.to_string().contains("project_id"), "the reason names what is missing: {got}");
}

/// A backend error of the memory adapter is not about the message: here the adapter refuses a history row for a work item that is not
/// there (a foreign-key failure, `DbError::Backend`), which reaches the consumer as `CoreError::Internal`. It is transient, which is the
/// known misclassification `consumer_error` documents: delivered again, it fails the same way, and is dead-lettered with its reason.
#[test]
fn a_backend_error_of_the_memory_adapter_is_transient() {
    let w = world();
    let consumer = ActivityConsumer(w.app.clone());
    let payload = serde_json::json!({ "project_id": w.project_a, "history": history() });

    // the control: the same history on a work item that is there is written, so the error below is the adapter's refusal and not the event
    let item = existing_work_item(&w);
    block_on(consumer.handle(&event(&w, 0xE3, item, payload.clone()))).unwrap();

    let got = block_on(consumer.handle(&event(&w, 0xE4, Uuid::from_u128(0xE5), payload))).unwrap_err();
    assert_eq!(kind(&got), Kind::Transient, "{got:?}");
    assert!(got.to_string().contains("work_item_activities_work_item_fkey"), "the reason is the adapter's own: {got}");
}
