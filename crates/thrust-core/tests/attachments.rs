//! Attachments (ADR-0080) through the use-cases: storage round trip, the size limit, and who may read and write.

mod common;

use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use rstest::rstest;
use thrust_adapters_memory::{ConsumerSlot, MemoryQueue};
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const LIMIT: usize = 8;

/// A world with a small attachment limit and one live work item in project A.
fn setup() -> (World, WorkItemId) {
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let queue = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    let cfg = CoreConfig { outbox_fastpath: false, max_attachment_bytes: LIMIT, ..CoreConfig::default() };
    let w = world_with(cfg, queue, background);
    let req = CreateWorkItemRequest { name: "item".into(), description: None, external_source: None, external_id: None };
    let item = block_on(create_work_item(&w.app, &w.principal(Actor::MemberA), &w.slug_a, w.project_a, req)).unwrap();
    (w, item.id)
}

fn put(w: &World, a: Actor, item: WorkItemId, name: &str, bytes: &[u8]) -> Result<(), CoreError> {
    block_on(put_attachment(&w.app, &w.principal(a), &w.slug_a, w.project_a, item, name, bytes.to_vec(), Some("text/plain".into())))
}

fn get(w: &World, a: Actor, item: WorkItemId, name: &str) -> Result<Blob, CoreError> {
    block_on(get_attachment(&w.app, &w.principal(a), &w.slug_a, w.project_a, item, name))
}

#[test]
fn an_attachment_round_trips_and_delete_removes_it() {
    let (w, item) = setup();
    put(&w, Actor::MemberA, item, "notes.txt", b"hello").unwrap();
    let blob = get(&w, Actor::GuestA, item, "notes.txt").unwrap();
    assert_eq!(blob, Blob { bytes: b"hello".to_vec(), content_type: Some("text/plain".into()) });

    block_on(delete_attachment(&w.app, &w.principal(Actor::MemberA), &w.slug_a, w.project_a, item, "notes.txt")).unwrap();
    assert_eq!(outcome(&get(&w, Actor::GuestA, item, "notes.txt")), Outcome::NotFound);
}

#[rstest]
#[case::empty(0, Outcome::Ok)]
#[case::one_under(LIMIT - 1, Outcome::Ok)]
#[case::at_the_limit(LIMIT, Outcome::Ok)]
#[case::one_over(LIMIT + 1, Outcome::TooLarge)]
fn the_size_limit_is_inclusive(#[case] len: usize, #[case] want: Outcome) {
    let (w, item) = setup();
    assert_eq!(outcome(&put(&w, Actor::MemberA, item, "f", &vec![b'x'; len])), want);
}

#[test]
fn an_attachment_needs_a_live_work_item() {
    let (w, item) = setup();
    let unknown = WorkItemId(Uuid::from_u128(0xF00));
    assert_eq!(outcome(&put(&w, Actor::MemberA, unknown, "f", b"x")), Outcome::NotFound);

    block_on(delete_work_item(&w.app, &w.principal(Actor::MemberA), &w.slug_a, w.project_a, item)).unwrap();
    assert_eq!(outcome(&put(&w, Actor::MemberA, item, "f", b"x")), Outcome::NotFound, "deleted items take no attachments");
}

#[rstest]
#[case::guest_may_not_write(Actor::GuestA, Outcome::Forbidden)]
#[case::outsider(Actor::OutsiderA, Outcome::Forbidden)]
#[case::other_tenant(Actor::MemberB, Outcome::NotFound)]
fn writing_needs_member_in_the_same_tenant(#[case] who: Actor, #[case] want: Outcome) {
    let (w, item) = setup();
    assert_eq!(outcome(&put(&w, who, item, "f", b"x")), want);
    let del = block_on(delete_attachment(&w.app, &w.principal(who), &w.slug_a, w.project_a, item, "f"));
    assert_eq!(outcome(&del), want);
}

#[test]
fn another_tenant_cannot_read_an_attachment() {
    let (w, item) = setup();
    put(&w, Actor::MemberA, item, "f", b"secret").unwrap();
    assert_eq!(outcome(&get(&w, Actor::MemberB, item, "f")), Outcome::NotFound);
    assert_eq!(outcome(&get(&w, Actor::OutsiderA, item, "f")), Outcome::Forbidden);
}
