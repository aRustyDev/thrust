//! Retention in the memory adapter (issue #59, F-D10, deviation D-001): `purgeable_work_items`, `purge_work_items`, `purge_outbox` and
//! `work_item_deleted_at`. The Postgres adapter is held to the same answers by the tests of the same names in
//! `crates/adapters/selfhosted/tests/pg_adapter.rs`.
//!
//! The rules, written out. A row is old when its time is strictly before the cutoff (Plane's `deleted_at < now - days`), so a row of
//! exactly the window's age stays.
//!
//! * A work item is purgeable when it was deleted before the cutoff and so was every work item under it (the pin lets a work item be
//!   its own parent, and a loop: each is old, so each goes). A live work item, or one deleted since, under a deleted work item keeps it.
//! * `purgeable_work_items` lists them by (`deleted_at`, `id`), after the key it is given, at most `limit`, whatever their tenant.
//! * `purge_work_items` deletes those of `ids` that are purgeable when it runs, and with them their history (D-043); nothing else.
//!   The `activity` projection and the project's counters stay, so a number is not handed out again (ADR-0043 B1).
//! * `purge_outbox` deletes the oldest `limit` rows that were dispatched and made before the cutoff, and never a row that was not
//!   dispatched.
//! * `work_item_deleted_at` is `None` for a work item that is not there, `Some(None)` for a live one, `Some(Some(at))` for a deleted one.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::BTreeSet;

use common::proptest_config;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;

const A: u128 = 0xA;
const B: u128 = 0xB;
const P: u128 = 0x10;
const R: u128 = 0x12;
/// The cutoff, in seconds from the base time of `at`.
const CUTOFF: i64 = 100_000;

fn wid(n: u128) -> WorkspaceId {
    WorkspaceId(Uuid::from_u128(n))
}

fn pid(n: u128) -> ProjectId {
    ProjectId(Uuid::from_u128(n))
}

fn uid(n: u128) -> UserId {
    UserId(Uuid::from_u128(n))
}

fn at(n: i64) -> Timestamp {
    DateTime::from_timestamp(1_760_000_000 + n, 0).unwrap()
}

fn cutoff() -> Timestamp {
    at(CUTOFF)
}

fn sys() -> SystemScope {
    SystemScope::mint()
}

/// The workspace of a project of this file: P is workspace A's, R is workspace B's.
fn workspace_of(project: u128) -> u128 {
    if project == R {
        B
    } else {
        A
    }
}

fn scope(project: u128) -> Scope {
    Scope::mint(wid(workspace_of(project)), pid(project), uid(1))
}

/// Workspace A holds project P and workspace B holds project R, so that the job is seen to reach both tenants.
fn world() -> MemoryDatabase {
    let db = MemoryDatabase::new();
    let f = SeedFixture {
        workspaces: vec![
            SeedWorkspace { id: wid(A), slug: WorkspaceSlug::new("a") },
            SeedWorkspace { id: wid(B), slug: WorkspaceSlug::new("b") },
        ],
        users: (1..=3).map(uid).collect(),
        projects: vec![
            SeedProject { id: pid(P), workspace_id: wid(A), name: "p".into() },
            SeedProject { id: pid(R), workspace_id: wid(B), name: "r".into() },
        ],
        ..SeedFixture::default()
    };
    block_on(db.seed(&sys(), &f)).unwrap();
    db
}

/// A live work item of `project`, numbered from its counter, made in a transaction of its own.
fn item(db: &MemoryDatabase, project: u128) -> WorkItemId {
    let s = scope(project);
    let mut tx = block_on(db.begin(&s)).unwrap();
    let a = block_on(tx.allocate(&s)).unwrap();
    let w = WorkItem {
        id: WorkItemId(Uuid::from_u128(0x9000 + (project << 16) + a.sequence_id as u128)),
        workspace_id: s.workspace_id(),
        project_id: pid(project),
        sequence_id: a.sequence_id,
        sort_order: a.sort_order,
        name: format!("item {}", a.sequence_id),
        description: None,
        external_source: None,
        external_id: None,
        created_by: Some(uid(1)),
        created_at: at(0),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    block_on(tx.insert_work_item(&s, &w)).unwrap();
    block_on(tx.insert_work_item_attrs(&s, w.id, &WorkItemAttrs::bare(w.created_at))).unwrap();
    block_on(tx.commit()).unwrap();
    w.id
}

/// Give a live work item of P this parent (any work item of P: itself and a loop too), keeping what else it has.
fn set_parent(db: &MemoryDatabase, id: WorkItemId, parent: WorkItemId) {
    let s = scope(P);
    let r = block_on(db.work_item_row(&s, id)).unwrap().unwrap();
    let u = WorkItemUpdate {
        name: r.item.name.clone(),
        state_id: r.attrs.state_id,
        priority: r.attrs.priority,
        start_date: r.attrs.start_date,
        target_date: r.attrs.target_date,
        completed_at: r.attrs.completed_at,
        updated_at: at(1),
        updated_by: uid(1),
        description_html: r.attrs.description_html.clone(),
        description_stripped: r.attrs.description_stripped.clone(),
        description_schema: r.attrs.description_schema,
        assignee_ids: None,
        label_ids: None,
        external_source: None,
        external_id: None,
        parent_id: Some(parent.0),
        point: None,
    };
    let mut tx = block_on(db.begin(&s)).unwrap();
    assert!(block_on(tx.update_work_item(&s, id, &u)).unwrap());
    block_on(tx.commit()).unwrap();
}

/// Delete the work item, and the live work items under it, at `when`; `false` when it was not live.
fn delete(db: &MemoryDatabase, project: u128, id: WorkItemId, when: Timestamp) -> bool {
    let s = scope(project);
    let mut tx = block_on(db.begin(&s)).unwrap();
    let found = block_on(tx.soft_delete_work_item(&s, id, when)).unwrap();
    block_on(tx.commit()).unwrap();
    found
}

/// What the table has as the work item's `deleted_at`, read by the adapter's own read of deleted rows.
fn deleted_at(db: &MemoryDatabase, project: u128, id: WorkItemId) -> Option<Timestamp> {
    block_on(db.get_work_item(&scope(project), id, Visibility::WithDeleted)).unwrap().and_then(|w| w.deleted_at)
}

fn there(db: &MemoryDatabase, project: u128, id: WorkItemId) -> bool {
    block_on(db.get_work_item(&scope(project), id, Visibility::WithDeleted)).unwrap().is_some()
}

fn candidates(db: &MemoryDatabase, after: Option<(Timestamp, WorkItemId)>, limit: u32) -> Vec<PurgeCandidate> {
    block_on(db.purgeable_work_items(&sys(), cutoff(), after, limit)).unwrap()
}

fn purge(db: &MemoryDatabase, ids: &[WorkItemId]) -> u64 {
    block_on(db.purge_work_items(&sys(), ids, cutoff())).unwrap()
}

/// One history row of the work item, of an event of its own.
fn history_row(project: u128, id: WorkItemId, event: u128) -> WorkItemActivity {
    WorkItemActivity {
        id: ActivityId(Uuid::from_u128(0xAC00_0000 + event)),
        event_id: EventId(Uuid::from_u128(event)),
        ordinal: 0,
        workspace_id: wid(workspace_of(project)),
        project_id: pid(project),
        work_item_id: id,
        actor_id: Some(uid(1)),
        verb: "created".into(),
        field: None,
        old_value: None,
        new_value: None,
        comment: "created the work item".into(),
        old_identifier: None,
        new_identifier: None,
        epoch: 1_760_000_000,
        created_at: at(0),
    }
}

fn history_of(db: &MemoryDatabase, project: u128, id: WorkItemId) -> Vec<WorkItemActivity> {
    block_on(db.work_item_history(&scope(project), id, None)).unwrap()
}

// --- the work items -------------------------------------------------------------------------------------------------------------

/// A forest of work items of P (each one's parent is any work item, itself and a loop included, or none), then deletions of work items
/// at times within two seconds of the cutoff, or long before it. A deletion takes the live work items under the one it names, so
/// that a deleted work item may have children that went before it or after it, in any order.
fn forest() -> impl Strategy<Value = (Vec<Option<usize>>, Vec<(usize, i64)>, u32)> {
    (1usize..9).prop_flat_map(|n| {
        (
            prop::collection::vec(prop::option::of(0..n), n),
            prop::collection::vec((0..n, prop_oneof![3 => -2i64..=2, 1 => Just(-50_000i64)]), 0..2 * n),
            1u32..4,
        )
    })
}

proptest! {
    #![proptest_config(proptest_config(96))]

    /// Over any forest and any deletions: the purgeable work items are exactly those that were deleted before the cutoff and whose
    /// children were too, in the order of the key, whole or a page at a time; purging them deletes them and their
    /// history and nothing else, and purging again deletes nothing. The model is the rule, computed from the deletion times the table
    /// has (which the cascade of a deletion makes, so they are read and not guessed).
    #[test]
    fn prop_the_purge_takes_exactly_what_the_rule_says((parents, deletions, page) in forest()) {
        let db = world();
        let ids: Vec<WorkItemId> = (0..parents.len()).map(|_| item(&db, P)).collect();
        for (id, parent) in ids.iter().zip(&parents) {
            if let Some(p) = parent {
                set_parent(&db, *id, ids[*p]);
            }
        }
        for (n, id) in ids.iter().enumerate() {
            prop_assert_eq!(block_on(db.insert_work_item_activities(&sys(), &[history_row(P, *id, 0xE000 + n as u128)])).unwrap(), 1);
        }
        for (i, secs) in &deletions {
            delete(&db, P, ids[*i], at(CUTOFF + secs));
        }

        let times: Vec<Option<Timestamp>> = ids.iter().map(|id| deleted_at(&db, P, *id)).collect();
        let old = |t: Option<Timestamp>| t.is_some_and(|t| t < cutoff());
        let expected: BTreeSet<usize> = (0..ids.len())
            .filter(|&i| old(times[i]) && (0..ids.len()).filter(|&c| parents[c] == Some(i)).all(|c| old(times[c])))
            .collect();

        // what is listed: the work items of the rule, by the key
        let all = candidates(&db, None, 100);
        let listed: BTreeSet<usize> = all.iter().map(|c| ids.iter().position(|id| *id == c.id).unwrap()).collect();
        prop_assert_eq!(&listed, &expected, "the work items deleted before the cutoff whose children were too");
        let keys: Vec<(Timestamp, WorkItemId)> = all.iter().map(|c| (c.deleted_at, c.id)).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        prop_assert_eq!(&keys, &sorted, "in the order of (deleted_at, id)");
        for c in &all {
            prop_assert_eq!((c.workspace_id, c.project_id), (wid(A), pid(P)));
            prop_assert_eq!(Some(c.deleted_at), deleted_at(&db, P, c.id));
        }
        // a page at a time, each from the key of the one before, is the same list
        let mut paged = Vec::new();
        let mut after = None;
        loop {
            let got = candidates(&db, after, page);
            prop_assert!(got.len() <= page as usize);
            let Some(last) = got.last() else { break };
            after = Some((last.deleted_at, last.id));
            paged.extend(got);
        }
        prop_assert_eq!(&paged, &all);

        // what is purged: those, and their history, and no other
        prop_assert_eq!(purge(&db, &ids), expected.len() as u64);
        for (i, id) in ids.iter().enumerate() {
            prop_assert_eq!(there(&db, P, *id), !expected.contains(&i), "item {}", i);
            prop_assert_eq!(history_of(&db, P, *id).len(), usize::from(!expected.contains(&i)), "the history of item {}", i);
        }
        prop_assert_eq!(purge(&db, &ids), 0, "a second run has nothing to do");
        prop_assert!(candidates(&db, None, 100).is_empty());
    }
}

/// For a work item of either tenant deleted `secs` seconds from the cutoff: it is purgeable, and goes, exactly when that is before the
/// cutoff, and what is listed names the work item's own workspace and project. The three answers of `work_item_deleted_at` are seen on
/// the way.
#[rstest]
fn a_work_item_goes_only_when_it_was_deleted_strictly_before_the_cutoff(
    #[values(-2, -1, 0, 1, 2)] secs: i64,
    #[values(P, R)] project: u128,
) {
    let db = world();
    let id = item(&db, project);
    assert_eq!(block_on(db.work_item_deleted_at(&sys(), id)).unwrap(), Some(None), "live");
    assert!(candidates(&db, None, 10).is_empty(), "a live work item is never purgeable");
    assert_eq!(purge(&db, &[id]), 0);

    assert!(delete(&db, project, id, at(CUTOFF + secs)));
    assert_eq!(block_on(db.work_item_deleted_at(&sys(), id)).unwrap(), Some(Some(at(CUTOFF + secs))), "deleted");
    let goes = secs < 0;
    let want: Vec<PurgeCandidate> = goes
        .then_some(PurgeCandidate {
            workspace_id: wid(workspace_of(project)),
            project_id: pid(project),
            id,
            deleted_at: at(CUTOFF + secs),
        })
        .into_iter()
        .collect();
    assert_eq!(candidates(&db, None, 10), want, "a work item deleted {secs} s from the cutoff");
    assert_eq!(purge(&db, &[id]), u64::from(goes));
    assert_eq!(
        block_on(db.work_item_deleted_at(&sys(), id)).unwrap(),
        if goes { None } else { Some(Some(at(CUTOFF + secs))) }
    );
}

/// Spec example, ADR-0043 B1: a purged work item's number is never handed out again, because the counter is not touched.
#[test]
fn a_number_is_not_handed_out_again_once_its_work_item_is_gone() {
    let db = world();
    let ids: Vec<WorkItemId> = (0..3).map(|_| item(&db, P)).collect();
    for id in &ids {
        assert!(delete(&db, P, *id, at(CUTOFF - 5)));
    }
    assert_eq!(purge(&db, &ids), 3);
    let s = scope(P);
    let mut tx = block_on(db.begin(&s)).unwrap();
    let next = block_on(tx.allocate(&s)).unwrap();
    assert_eq!(next.sequence_id, 4, "the three numbers that went are not made again");
    block_on(tx.rollback()).unwrap();
}

/// Spec example, deviation D-043 (the owner's decision of 2026-11-19, where Plane keeps the history): a purged work item's history goes
/// with it, so a late write for it is refused as one for a work item that is not there. The key that made the writes idempotent goes
/// too, as Postgres frees `UNIQUE (event_id, ordinal)` with the row: the same key may be written again, for another work item. The
/// `activity` projection, which has no foreign key to the work item, stays.
#[test]
fn d043_a_purged_work_items_history_goes_with_it_and_a_late_write_is_refused() {
    let db = world();
    let (gone, kept) = (item(&db, P), item(&db, P));
    let (row_gone, row_kept) = (history_row(P, gone, 0xE1), history_row(P, kept, 0xE2));
    assert_eq!(block_on(db.insert_work_item_activities(&sys(), &[row_gone.clone(), row_kept])).unwrap(), 2);
    let projection = Activity {
        id: ActivityId(Uuid::from_u128(0xAC1)),
        event_id: EventId(Uuid::from_u128(0xE1)),
        workspace_id: wid(A),
        project_id: pid(P),
        entity_id: gone.0,
        verb: "created".into(),
        actor_id: Some(uid(1)),
        created_at: at(0),
    };
    assert!(block_on(db.insert_activity(&sys(), &projection)).unwrap());

    assert!(delete(&db, P, gone, at(CUTOFF - 1)));
    assert_eq!(purge(&db, &[gone, kept]), 1, "the live one stays");

    assert!(history_of(&db, P, gone).is_empty());
    assert_eq!(history_of(&db, P, kept).len(), 1);
    assert!(
        matches!(
            block_on(db.insert_work_item_activities(&sys(), std::slice::from_ref(&row_gone))),
            Err(DbError::Backend(_))
        ),
        "a late write for a purged work item is refused"
    );
    let again = WorkItemActivity { id: ActivityId(Uuid::from_u128(0xAC9)), work_item_id: kept, ..row_gone };
    assert_eq!(
        block_on(db.insert_work_item_activities(&sys(), &[again])).unwrap(),
        1,
        "the key of a purged row is free"
    );
    assert_eq!(block_on(db.list_activity(&scope(P), Some(gone))).unwrap(), vec![projection]);
}

/// Spec example for the rule's second half: a work item whose child is not old stays, though it is old itself, and goes with the child
/// once that is old too. (The forest property reaches the same case; this one names it.)
#[test]
fn a_work_item_with_a_child_that_is_not_old_stays_and_goes_with_the_child() {
    let db = world();
    let (parent, child) = (item(&db, P), item(&db, P));
    set_parent(&db, child, parent);
    // the child is deleted since the cutoff, the parent long before it (the child went first, so the parent's deletion passes it by)
    assert!(delete(&db, P, child, at(CUTOFF + 1)));
    assert!(delete(&db, P, parent, at(CUTOFF - 1)));
    assert!(candidates(&db, None, 10).is_empty(), "a work item with a child that is not old stays");
    assert_eq!(purge(&db, &[parent, child]), 0, "a work item with a child that is not old stays");
    // a cutoff after both takes them together, the parent first in the key (it was deleted first)
    let later = at(CUTOFF + 2);
    let got = block_on(db.purgeable_work_items(&sys(), later, None, 10)).unwrap();
    assert_eq!(got.iter().map(|c| c.id).collect::<Vec<_>>(), [parent, child]);
    assert_eq!(block_on(db.purge_work_items(&sys(), &[parent, child], later)).unwrap(), 2);
}

// --- the outbox -----------------------------------------------------------------------------------------------------------------

/// A row of the outbox of the project, made `created` seconds from the base time and dispatched or not.
fn outbox_row(project: u128, event: u128, created: i64, dispatched: bool) -> OutboxRow {
    OutboxRow {
        envelope: Envelope {
            event_id: EventId(Uuid::from_u128(event)),
            schema_version: 1,
            workspace_id: wid(workspace_of(project)),
            event_type: "work_item.created".into(),
            entity_type: "work_item".into(),
            entity_id: Uuid::from_u128(0x5000 + event),
            actor_id: None,
            occurred_at: at(created),
            payload: serde_json::json!({ "project_id": pid(project) }),
            trace_id: None,
        },
        created_at: at(created),
        dispatched_at: dispatched.then(|| at(created + 1)),
        attempts: i32::from(dispatched),
    }
}

fn outbox_insert(db: &MemoryDatabase, row: &OutboxRow) {
    let project = if row.envelope.workspace_id == wid(B) { R } else { P };
    let s = scope(project);
    let mut tx = block_on(db.begin(&s)).unwrap();
    block_on(tx.insert_outbox(&s, row)).unwrap();
    block_on(tx.commit()).unwrap();
}

fn purge_outbox(db: &MemoryDatabase, cutoff: Timestamp, limit: u32) -> u64 {
    block_on(db.purge_outbox(&sys(), cutoff, limit)).unwrap()
}

proptest! {
    #![proptest_config(proptest_config(96))]

    /// Over rows of both tenants, each made a second after the one before in age (so no two share a time), some dispatched, written in
    /// any order, with the cutoff between any two of them or on one: `purge_outbox` deletes the oldest `limit` of the dispatched rows
    /// made before the cutoff, and no other. What is left is read by purges of the same table (there is no other read of a dispatched
    /// row): one that reaches just past the oldest row it was meant to leave finds none older, and one that reaches past every row
    /// finds all that are left.
    #[test]
    fn prop_the_outbox_purge_takes_the_oldest_dispatched_rows_before_the_cutoff(
        (dispatched, order, boundary, limit) in (0usize..12).prop_flat_map(|n| (
            prop::collection::vec(any::<bool>(), n),
            Just((0..n).collect::<Vec<_>>()).prop_shuffle(),
            0..=n,
            0u32..14,
        )),
    ) {
        let db = world();
        // row `i` is `i - boundary` seconds from the cutoff: those before `boundary` are old, `boundary` is on it, the rest are newer
        let created = |i: usize| CUTOFF + i as i64 - boundary as i64;
        for i in order {
            let project = if i % 2 == 0 { P } else { R };
            outbox_insert(&db, &outbox_row(project, 0xB000 + i as u128, created(i), dispatched[i]));
        }
        let waiting: Vec<OutboxRow> = block_on(db.undispatched_outbox(&sys(), 100)).unwrap();

        let old: Vec<usize> = (0..dispatched.len()).filter(|&i| dispatched[i] && i < boundary).collect();
        let taken = old.len().min(limit as usize);
        prop_assert_eq!(
            purge_outbox(&db, cutoff(), limit),
            taken as u64,
            "the outbox purge: the oldest dispatched rows made before the cutoff, at most the limit"
        );

        prop_assert_eq!(
            block_on(db.undispatched_outbox(&sys(), 100)).unwrap(),
            waiting,
            "the outbox purge: a row that was not dispatched is never touched"
        );
        if let Some(&newest_taken) = taken.checked_sub(1).and_then(|last| old.get(last)) {
            prop_assert_eq!(
                purge_outbox(&db, at(created(newest_taken) + 1), u32::MAX),
                0,
                "the outbox purge: nothing older than the last one taken is left"
            );
        }
        let dispatched_rows = dispatched.iter().filter(|d| **d).count();
        prop_assert_eq!(
            purge_outbox(&db, at(CUTOFF + 1_000), u32::MAX),
            (dispatched_rows - taken) as u64,
            "the outbox purge: the dispatched rows that are left, and no other"
        );
    }
}
