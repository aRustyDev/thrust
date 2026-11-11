//! A work item's history and its update in the memory adapter (issue #44, stage 6b, migration 0014). The Postgres adapter is held to the
//! same answers by the generated sequences in `thrust-adapters-selfhosted/tests/model` and the examples in its `pg_adapter.rs`.
//!
//! The rules, written out:
//!
//! * `insert_work_item_activities` writes rows of the history of work items, idempotently on (`event_id`, `ordinal`): a row whose key is
//!   there is left as it is (so is the second of two rows of one batch with one key), and the answer is how many rows were written. A
//!   row whose work item is not in the workspace AND project it names, or whose actor is no user, is refused (`Backend`), a row whose
//!   key is there as well, and a batch that holds one writes none of its rows.
//! * `work_item_history` is the rows of one work item of the scope's workspace and project, oldest first, the rows of one time in the
//!   order they were written; with `after`, only those made strictly later.
//! * `latest_work_item_activity` is the last of those (the newest by time, then the one written last) for the workspace, project and
//!   work item it names.
//! * `touch_work_item_activity` moves a row's time and nothing else (the row keeps its place among those written); `false` for a row
//!   that is not there.
//! * `update_work_item` writes the name and the attributes it names to a live work item of the scope's project (not deleted, archived
//!   or a draft): the people and the labels are replaced, sorted and once each, only when they are named; the editor is recorded. What
//!   the Postgres constraints would refuse (a state, an editor, a person or a label that does not exist; a start after a target) is a
//!   `Backend` error and writes nothing; an item that is not there is `false` (and the payload is not looked at); nothing is seen before
//!   the transaction commits, and nothing is left by one that does not.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::sync::atomic::{AtomicU64, Ordering};

use common::proptest_config;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::chrono::{DateTime, NaiveDate, TimeDelta};
use thrust_contracts::*;
use thrust_ports::*;

/// Workspace A holds projects P and Q; workspace B holds R.
const A: u128 = 0xA;
const B: u128 = 0xB;
const P: u128 = 0x10;
const Q: u128 = 0x11;
const R: u128 = 0x12;

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

fn day(n: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 1).unwrap() + TimeDelta::days(n)
}

fn sys() -> SystemScope {
    SystemScope::mint()
}

fn scope(ws: u128, project: u128) -> Scope {
    Scope::mint(wid(ws), pid(project), uid(1))
}

/// The four states of project P, one for each group but two.
const STATES: [(u128, StateGroup); 4] = [
    (0x501, StateGroup::Backlog),
    (0x502, StateGroup::Unstarted),
    (0x503, StateGroup::Started),
    (0x504, StateGroup::Completed),
];

fn state_id(i: usize) -> Uuid {
    Uuid::from_u128(STATES[i].0)
}

/// The labels of P are 1 to 3; Q has one more, which is not P's.
fn label_id(n: u128) -> Uuid {
    Uuid::from_u128(0x700 + n)
}

/// A state of project Q.
const ELSEWHERE: u128 = 0x5FF;

/// Workspace A with users 1 to 4 (5 is a user in no workspace, 99 is nobody), project P (user 1 its admin, 2 a member, 3 a guest; 4 is
/// in the workspace and not in the project) with its four states and three labels, project Q with a state and a label of its own, and
/// workspace B with project R.
fn world() -> MemoryDatabase {
    let db = MemoryDatabase::new();
    let state = |project: u128, ws: u128, n: u128, sequence: f64, group: StateGroup, is_default: bool| SeedState {
        id: Uuid::from_u128(n),
        project_id: pid(project),
        workspace_id: wid(ws),
        name: format!("state {n}"),
        color: "#000000".into(),
        sequence,
        group,
        is_default,
    };
    let mut states: Vec<SeedState> = STATES
        .iter()
        .enumerate()
        .map(|(i, (id, group))| state(P, A, *id, 10_000.0 * (i as f64 + 1.0), *group, i == 0))
        .collect();
    states.push(state(Q, A, ELSEWHERE, 1.0, StateGroup::Backlog, true));
    let label = |project: u128, n: u128, id: Uuid| SeedLabel {
        id,
        project_id: pid(project),
        workspace_id: wid(A),
        name: format!("l{project}-{n}"),
        color: "#fff".into(),
        parent_id: None,
        sort_order: n as f64,
    };
    let f = SeedFixture {
        workspaces: vec![
            SeedWorkspace { id: wid(A), slug: WorkspaceSlug::new("a") },
            SeedWorkspace { id: wid(B), slug: WorkspaceSlug::new("b") },
        ],
        users: (1..=5).map(uid).collect(),
        workspace_members: (1..=4).map(|u| SeedWorkspaceMember { workspace_id: wid(A), user_id: uid(u) }).collect(),
        projects: vec![
            SeedProject { id: pid(P), workspace_id: wid(A), name: "p".into() },
            SeedProject { id: pid(Q), workspace_id: wid(A), name: "q".into() },
            SeedProject { id: pid(R), workspace_id: wid(B), name: "r".into() },
        ],
        memberships: [(1, Role::Admin), (2, Role::Member), (3, Role::Guest)]
            .iter()
            .map(|(u, role)| SeedMembership { project_id: pid(P), workspace_id: wid(A), user_id: uid(*u), role: *role })
            .collect(),
        states,
        labels: vec![
            label(P, 1, label_id(1)),
            label(P, 2, label_id(2)),
            label(P, 3, label_id(3)),
            label(Q, 1, Uuid::from_u128(0x8FF)),
        ],
        ..SeedFixture::default()
    };
    block_on(db.seed(&sys(), &f)).unwrap();
    db
}

/// Insert one work item of `project` in a transaction of its own (`edit` changes what it is first), and commit it.
fn make_item(db: &MemoryDatabase, ws: u128, project: u128, edit: impl FnOnce(&mut WorkItem)) -> WorkItemId {
    let s = scope(ws, project);
    let mut tx = block_on(db.begin(&s)).unwrap();
    let a = block_on(tx.allocate(&s)).unwrap();
    let mut item = WorkItem {
        id: WorkItemId(Uuid::from_u128(0x9000 + (project << 16) + a.sequence_id as u128)),
        workspace_id: wid(ws),
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
    edit(&mut item);
    block_on(tx.insert_work_item(&s, &item)).unwrap();
    block_on(tx.commit()).unwrap();
    item.id
}

// --- the history --------------------------------------------------------------------------------------------------------------

/// A work item and the workspace and project it is in.
#[derive(Debug, Clone, Copy)]
struct Target {
    ws: u128,
    project: u128,
    item: WorkItemId,
}

/// Two work items of P (`x`, `y`), one of Q and one of R (in another workspace).
struct World {
    db: MemoryDatabase,
    x: Target,
    y: Target,
    z: Target,
    w: Target,
}

fn history_world() -> World {
    let db = world();
    let target = |ws, project| Target { ws, project, item: make_item(&db, ws, project, |_| {}) };
    let (x, y, z, w) = (target(A, P), target(A, P), target(A, Q), target(B, R));
    World { db, x, y, z, w }
}

static MINT: AtomicU64 = AtomicU64::new(1);

/// A row for `t` of event `event`, at place `ordinal` in it, made `secs` after one time; every id and value of it is its own, so a
/// row that is stored as another is seen.
fn row(t: Target, event: u128, ordinal: i32, secs: i64) -> WorkItemActivity {
    let n = u128::from(MINT.fetch_add(1, Ordering::Relaxed));
    WorkItemActivity {
        id: ActivityId(Uuid::from_u128(0xAC00_0000 + n)),
        event_id: EventId(Uuid::from_u128(event)),
        ordinal,
        workspace_id: wid(t.ws),
        project_id: pid(t.project),
        work_item_id: t.item,
        actor_id: Some(uid(1 + n % 3)),
        verb: if ordinal == 0 { "created" } else { "updated" }.into(),
        field: (ordinal != 0).then(|| format!("field {n}")),
        old_value: Some(format!("old {n}")),
        new_value: (n % 2 == 0).then(|| format!("new {n}")),
        comment: format!("comment {n}"),
        old_identifier: Some(Uuid::from_u128(0x1D00 + n)),
        new_identifier: (n % 3 == 0).then(|| Uuid::from_u128(0x2D00 + n)),
        epoch: 1_760_000_000 + secs,
        created_at: at(secs),
    }
}

fn insert(db: &MemoryDatabase, rows: &[WorkItemActivity]) -> Result<u64, DbError> {
    block_on(db.insert_work_item_activities(&sys(), rows))
}

fn history(
    db: &MemoryDatabase,
    ws: u128,
    project: u128,
    item: WorkItemId,
    after: Option<Timestamp>,
) -> Vec<WorkItemActivity> {
    block_on(db.work_item_history(&scope(ws, project), item, after)).unwrap()
}

fn latest(db: &MemoryDatabase, ws: u128, project: u128, item: WorkItemId) -> Option<WorkItemActivity> {
    block_on(db.latest_work_item_activity(&sys(), wid(ws), pid(project), item)).unwrap()
}

/// Events of rows: each event is one batch, a row of it is (which of P's two items, how many seconds after one time). Few times, so
/// that rows tie.
fn events() -> impl Strategy<Value = Vec<Vec<(usize, i64)>>> {
    prop::collection::vec(prop::collection::vec((0..2usize, 0..4i64), 1..4), 1..5)
}

/// Write every event as a batch of its own (event `0xE000 + e`, a row's place in the batch is its ordinal) and answer the rows in the
/// order they were written.
fn write_events(w: &World, events: &[Vec<(usize, i64)>]) -> Vec<WorkItemActivity> {
    let mut written = Vec::new();
    for (e, rows) in events.iter().enumerate() {
        let batch: Vec<WorkItemActivity> = rows
            .iter()
            .enumerate()
            .map(|(ordinal, (item, secs))| row([w.x, w.y][*item], 0xE000 + e as u128, ordinal as i32, *secs))
            .collect();
        assert_eq!(insert(&w.db, &batch).unwrap(), batch.len() as u64);
        written.extend(batch);
    }
    written
}

/// The model the adapter is held to: the rows of `item` (made strictly after `after`, if given), by time and then by the place they
/// were written at.
fn expected_history(written: &[WorkItemActivity], item: WorkItemId, after: Option<Timestamp>) -> Vec<WorkItemActivity> {
    let mut v: Vec<(usize, &WorkItemActivity)> = written
        .iter()
        .enumerate()
        .filter(|(_, a)| a.work_item_id == item && after.is_none_or(|t| a.created_at > t))
        .collect();
    v.sort_by_key(|(place, a)| (a.created_at, *place));
    v.into_iter().map(|(_, a)| a.clone()).collect()
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// The history of an item is its rows oldest first, and rows of one time stay in the order they were written; `after` keeps the
    /// rows made strictly later, whether it falls before every row, on a row's time or after them all.
    #[test]
    fn prop_history_is_oldest_first_with_ties_in_the_order_written(events in events(), after in prop::option::of(-1..5i64)) {
        let w = history_world();
        let written = write_events(&w, &events);
        for item in [w.x.item, w.y.item] {
            let got = history(&w.db, A, P, item, after.map(at));
            prop_assert_eq!(got, expected_history(&written, item, after.map(at)));
        }
    }

    /// The newest row of an item is the last of its history: the latest time, and of those the one written last.
    #[test]
    fn prop_latest_is_the_last_of_the_history(events in events()) {
        let w = history_world();
        let written = write_events(&w, &events);
        for item in [w.x.item, w.y.item] {
            prop_assert_eq!(latest(&w.db, A, P, item), expected_history(&written, item, None).pop());
        }
    }

    /// A touch moves one row's time and nothing else: the row is the same but for the time, it keeps its place among the rows written
    /// (so it now ties or not with others), and the history and the newest row follow.
    #[test]
    fn prop_touch_moves_the_time_and_nothing_else(events in events(), pick in 0..64usize, secs in -1..7i64) {
        let w = history_world();
        let mut written = write_events(&w, &events);
        let k = pick % written.len();
        prop_assert!(block_on(w.db.touch_work_item_activity(&sys(), written[k].id, at(secs))).unwrap());
        written[k].created_at = at(secs);
        for item in [w.x.item, w.y.item] {
            prop_assert_eq!(history(&w.db, A, P, item, None), expected_history(&written, item, None));
            prop_assert_eq!(latest(&w.db, A, P, item), expected_history(&written, item, None).pop());
        }
    }

    /// An event delivered twice writes nothing the second time, whatever the second delivery says (other ids, other values, another
    /// time): the rows stay as they were. A delivery that also holds a place the first did not writes that row alone, after the others.
    #[test]
    fn prop_a_redelivered_event_writes_nothing_and_leaves_the_rows_as_they_were(events in events(), pick in 0..8usize, extend in any::<bool>()) {
        let w = history_world();
        let mut written = write_events(&w, &events);
        let event = EventId(Uuid::from_u128(0xE000 + (pick % events.len()) as u128));
        let mut again: Vec<WorkItemActivity> = written
            .iter()
            .filter(|r| r.event_id == event)
            .map(|r| WorkItemActivity { id: ActivityId(Uuid::from_u128(0xBEEF_0000 + u128::from(MINT.fetch_add(1, Ordering::Relaxed)))), new_value: Some("again".into()), created_at: at(9), ..r.clone() })
            .collect();
        let places = again.len() as i32;
        if extend {
            again.push(row(w.x, event.0.as_u128(), places, 8));
        }
        let wrote = insert(&w.db, &again).unwrap();
        prop_assert_eq!(wrote, u64::from(extend));
        if extend {
            written.push(again.pop().unwrap());
        }
        for item in [w.x.item, w.y.item] {
            prop_assert_eq!(history(&w.db, A, P, item, None), expected_history(&written, item, None));
        }
    }

    /// Of two rows of one batch with one key, the first is written and the second is not.
    #[test]
    fn prop_the_first_of_two_rows_with_one_key_in_one_batch_wins(first in 0..4i64, second in 0..4i64) {
        let w = history_world();
        let (a, b) = (row(w.x, 0xE1, 0, first), row(w.x, 0xE1, 0, second));
        prop_assert_eq!(insert(&w.db, &[a.clone(), b]).unwrap(), 1);
        prop_assert_eq!(history(&w.db, A, P, w.x.item, None), vec![a]);
    }
}

/// A row that the adapter must refuse, by what is wrong with it.
#[derive(Debug, Clone, Copy)]
enum Stray {
    /// The work item is of project Q, the row names P.
    ItemOfAnotherProject,
    /// The work item is of workspace B, the row names A and P.
    ItemOfAnotherWorkspace,
    UnknownItem,
    /// The work item is P's, the row names Q (of the same workspace).
    NamesAnotherProject,
    /// The work item is P's, the row names workspace B (and P).
    NamesAnotherWorkspace,
    /// The work item is P's, the row names workspace B and its project R.
    NamesAnotherTenant,
    UnknownActor,
}

fn stray_row(w: &World, kind: Stray, event: u128) -> WorkItemActivity {
    let mut r = row(w.x, event, 0, 1);
    match kind {
        Stray::ItemOfAnotherProject => r.work_item_id = w.z.item,
        Stray::ItemOfAnotherWorkspace => r.work_item_id = w.w.item,
        Stray::UnknownItem => r.work_item_id = WorkItemId(Uuid::from_u128(0xBAD)),
        Stray::NamesAnotherProject => r.project_id = pid(Q),
        Stray::NamesAnotherWorkspace => r.workspace_id = wid(B),
        Stray::NamesAnotherTenant => (r.workspace_id, r.project_id) = (wid(B), pid(R)),
        Stray::UnknownActor => r.actor_id = Some(uid(99)),
    }
    r
}

/// A batch with one row that is refused, wherever it is among the others, is refused whole: nothing of it is written, the rows that
/// were there are as they were, and the rows around the bad one can be written afterwards (they were not).
#[rstest]
fn a_batch_with_a_refused_row_writes_none_of_its_rows(
    #[values(
        Stray::ItemOfAnotherProject,
        Stray::ItemOfAnotherWorkspace,
        Stray::UnknownItem,
        Stray::NamesAnotherProject,
        Stray::NamesAnotherWorkspace,
        Stray::NamesAnotherTenant,
        Stray::UnknownActor
    )]
    kind: Stray,
    #[values(0, 1, 2)] place: usize,
) {
    let w = history_world();
    let earlier = row(w.x, 0xE0, 0, 2);
    assert_eq!(insert(&w.db, std::slice::from_ref(&earlier)).unwrap(), 1);
    let valid = [row(w.x, 0xE2, 0, 3), row(w.y, 0xE2, 1, 3)];
    let mut batch = valid.to_vec();
    batch.insert(place, stray_row(&w, kind, 0xE3));
    let got = insert(&w.db, &batch);
    assert!(matches!(got, Err(DbError::Backend(_))), "{kind:?} at {place}: {got:?}");
    // nothing of it: not the rows around the bad one, and not the bad one under the tenant it names
    assert_eq!(history(&w.db, A, P, w.x.item, None), vec![earlier]);
    assert_eq!(history(&w.db, A, P, w.y.item, None), vec![]);
    for t in [w.z, w.w] {
        assert_eq!(history(&w.db, t.ws, t.project, t.item, None), vec![], "{kind:?}");
    }
    // so the valid rows are new: written now, both
    assert_eq!(insert(&w.db, &valid).unwrap(), 2);
}

/// A row whose key is there is refused all the same when it names a work item that is not there, or an actor who is no user: the rows
/// are asked about before any is skipped.
#[rstest]
fn a_row_whose_key_is_there_is_refused_when_it_is_a_stray(
    #[values(Stray::UnknownItem, Stray::UnknownActor, Stray::ItemOfAnotherProject)] kind: Stray,
) {
    let w = history_world();
    let first = row(w.x, 0xE0, 0, 2);
    assert_eq!(insert(&w.db, std::slice::from_ref(&first)).unwrap(), 1);
    let again = stray_row(&w, kind, 0xE0);
    let got = insert(&w.db, &[again]);
    assert!(matches!(got, Err(DbError::Backend(_))), "{kind:?}: {got:?}");
    assert_eq!(history(&w.db, A, P, w.x.item, None), vec![first]);
}

/// A row whose work item is deleted (soft) is still written: the history outlives the item's visibility. A row with no actor is written.
#[test]
fn a_row_for_a_deleted_work_item_and_a_row_with_no_actor_are_written() {
    let w = history_world();
    let s = scope(A, P);
    let mut tx = block_on(w.db.begin(&s)).unwrap();
    assert!(block_on(tx.soft_delete_work_item(&s, w.x.item, at(5))).unwrap());
    block_on(tx.commit()).unwrap();
    let mut r = row(w.x, 0xE0, 0, 1);
    r.actor_id = None;
    assert_eq!(insert(&w.db, &[r.clone()]).unwrap(), 1);
    assert_eq!(history(&w.db, A, P, w.x.item, None), vec![r]);
}

#[test]
fn an_empty_batch_writes_nothing() {
    let w = history_world();
    assert_eq!(insert(&w.db, &[]).unwrap(), 0);
}

/// The tenant a read names decides what it finds: the rows of an item are there for its own workspace and project and for no other
/// pair (nor for the project of an item of another project), and an item that is not there has none.
#[rstest]
fn a_read_finds_the_rows_of_the_tenant_it_names_alone(#[values(A, B)] ws: u128, #[values(P, Q, R)] project: u128) {
    let w = history_world();
    let rows =
        [row(w.x, 0xE0, 0, 1), row(w.x, 0xE0, 1, 2), row(w.y, 0xE1, 0, 1), row(w.z, 0xE2, 0, 1), row(w.w, 0xE3, 0, 1)];
    assert_eq!(insert(&w.db, &rows).unwrap(), 5);
    let own = ws == A && project == P;
    let x_rows = vec![rows[0].clone(), rows[1].clone()];
    assert_eq!(
        history(&w.db, ws, project, w.x.item, None),
        if own { x_rows } else { vec![] },
        "history of x as ({ws}, {project})"
    );
    assert_eq!(latest(&w.db, ws, project, w.x.item), own.then(|| rows[1].clone()), "latest of x as ({ws}, {project})");
    // an item of Q is not found through P, and neither is one that is not there
    assert_eq!(history(&w.db, A, P, w.z.item, None), vec![]);
    assert_eq!(latest(&w.db, A, P, w.z.item), None);
    let nobody = WorkItemId(Uuid::from_u128(0xBAD));
    assert_eq!(history(&w.db, ws, project, nobody, None), vec![]);
    assert_eq!(latest(&w.db, ws, project, nobody), None);
}

/// A row is touched by its id, wherever it is; a row that is not there answers `false` and changes nothing.
#[test]
fn touching_a_row_that_is_not_there_changes_nothing() {
    let w = history_world();
    let rows = [row(w.x, 0xE0, 0, 3), row(w.x, 0xE0, 1, 1)];
    assert_eq!(insert(&w.db, &rows).unwrap(), 2);
    assert!(!block_on(w.db.touch_work_item_activity(&sys(), ActivityId(Uuid::from_u128(0xBAD)), at(9))).unwrap());
    assert_eq!(history(&w.db, A, P, w.x.item, None), vec![rows[1].clone(), rows[0].clone()]);
}

/// The two rows of one time stay in the order they were written when the earlier one is touched to the later one's time (it
/// does not jump ahead) and when the later one is touched to the earlier one's (it does not fall behind).
#[test]
fn a_touched_row_keeps_its_place_among_rows_of_its_new_time() {
    let w = history_world();
    let (first, second) = (row(w.x, 0xE0, 0, 1), row(w.x, 0xE0, 1, 2));
    assert_eq!(insert(&w.db, &[first.clone(), second.clone()]).unwrap(), 2);
    // the earlier row moves to the later one's time: they tie, the earlier written stays first
    assert!(block_on(w.db.touch_work_item_activity(&sys(), first.id, at(2))).unwrap());
    let moved = WorkItemActivity { created_at: at(2), ..first.clone() };
    assert_eq!(history(&w.db, A, P, w.x.item, None), vec![moved.clone(), second.clone()]);
    assert_eq!(latest(&w.db, A, P, w.x.item), Some(second.clone()));
    // the later row moves to a time before both: it is the oldest now, and the newest is the other
    assert!(block_on(w.db.touch_work_item_activity(&sys(), second.id, at(0))).unwrap());
    let early = WorkItemActivity { created_at: at(0), ..second };
    assert_eq!(history(&w.db, A, P, w.x.item, None), vec![early, moved.clone()]);
    assert_eq!(latest(&w.db, A, P, w.x.item), Some(moved));
}

// --- the update ---------------------------------------------------------------------------------------------------------------

/// What an item made for these tests has before an update: every attribute set, so that an update's change shows in each.
fn before() -> WorkItemAttrs {
    WorkItemAttrs {
        state_id: Some(state_id(0)),
        priority: Priority::High,
        start_date: Some(day(1)),
        target_date: Some(day(2)),
        completed_at: Some(at(3)),
        updated_at: at(1),
        updated_by: Some(uid(3)),
        description_html: "<p>before</p>".into(),
        assignee_ids: vec![uid(1), uid(2)],
        label_ids: vec![label_id(1), label_id(2)],
        parent_id: None,
        point: Some(1),
    }
}

/// An item of P with `before()`'s attributes, in the state `edit` makes of it first.
fn made(db: &MemoryDatabase, edit: impl FnOnce(&mut WorkItem)) -> WorkItemId {
    let s = scope(A, P);
    let mut tx = block_on(db.begin(&s)).unwrap();
    let a = block_on(tx.allocate(&s)).unwrap();
    let mut item = WorkItem {
        id: WorkItemId(Uuid::from_u128(0x7000 + a.sequence_id as u128)),
        workspace_id: wid(A),
        project_id: pid(P),
        sequence_id: a.sequence_id,
        sort_order: a.sort_order,
        name: "before".into(),
        description: Some("the slice's description".into()),
        external_source: None,
        external_id: None,
        created_by: Some(uid(1)),
        created_at: at(0),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    edit(&mut item);
    block_on(tx.insert_work_item(&s, &item)).unwrap();
    block_on(tx.insert_work_item_attrs(&s, item.id, &before())).unwrap();
    block_on(tx.commit()).unwrap();
    item.id
}

/// An update that changes every column it can, and names people and labels.
fn change() -> WorkItemUpdate {
    WorkItemUpdate {
        name: "renamed".into(),
        state_id: Some(state_id(3)),
        priority: Priority::Urgent,
        start_date: Some(day(5)),
        target_date: Some(day(9)),
        completed_at: Some(at(40)),
        updated_at: at(50),
        updated_by: uid(2),
        description_html: "<p>after</p>".into(),
        assignee_ids: Some(vec![uid(3)]),
        label_ids: Some(vec![label_id(3)]),
        external_source: Some("gh".into()),
        external_id: Some("7".into()),
        parent_id: None,
        point: Some(12),
    }
}

/// `update_work_item` in a transaction of its own, committed whatever it answered.
fn update(db: &MemoryDatabase, s: &Scope, id: WorkItemId, u: &WorkItemUpdate) -> Result<bool, DbError> {
    let mut tx = block_on(db.begin(s)).unwrap();
    let got = block_on(tx.update_work_item(s, id, u));
    block_on(tx.commit()).unwrap();
    got
}

fn row_of(db: &MemoryDatabase, id: WorkItemId) -> Option<WorkItemRow> {
    block_on(db.work_item_row(&scope(A, P), id)).unwrap()
}

/// Who shows as assigned: the people who are in the workspace and the project (users 1 to 3), once each, in id order.
fn shown(people: &[UserId]) -> Vec<UserId> {
    let mut v: Vec<UserId> = people.iter().copied().filter(|u| (uid(1)..=uid(3)).contains(u)).collect();
    v.sort();
    v.dedup();
    v
}

/// The spec of an update, written from the port's documentation: the row as a read finds it once `u` has been written to it.
fn after(row: &WorkItemRow, u: &WorkItemUpdate) -> WorkItemRow {
    let mut r = row.clone();
    r.item.name = u.name.clone();
    r.attrs.state_id = u.state_id;
    r.attrs.priority = u.priority;
    r.attrs.start_date = u.start_date;
    r.attrs.target_date = u.target_date;
    r.attrs.completed_at = u.completed_at;
    r.attrs.updated_at = u.updated_at;
    r.attrs.updated_by = Some(u.updated_by);
    r.attrs.description_html = u.description_html.clone();
    r.attrs.parent_id = u.parent_id;
    r.attrs.point = u.point;
    r.item.external_source = u.external_source.clone();
    r.item.external_id = u.external_id.clone();
    if let Some(people) = &u.assignee_ids {
        r.attrs.assignee_ids = shown(people);
    }
    if let Some(labels) = &u.label_ids {
        let mut l = labels.clone();
        l.sort();
        l.dedup();
        r.attrs.label_ids = l;
    }
    r.state_group =
        u.state_id.and_then(|s| STATES.iter().find(|(id, _)| Uuid::from_u128(*id) == s)).map(|(_, group)| *group);
    r
}

/// An update that the constraints accept, of every shape: each column anything it can be, people and labels named or not (with
/// repeats, with people who show and people who do not).
fn any_update() -> impl Strategy<Value = WorkItemUpdate> {
    (
        "[ -~]{0,12}",
        prop::option::of(0..4usize),
        0..5usize,
        prop::option::of(0..6i64),
        prop::option::of(0..4i64),
        prop::option::of(0..4i64),
        0..9i64,
        1..=3u128,
        "[ -~]{0,12}",
        prop::option::of(prop::collection::vec(1..=5u128, 0..6)),
        prop::option::of(prop::collection::vec(1..=3u128, 0..6)),
    )
        .prop_map(|(name, state, priority, start, extra, completed, updated, by, html, people, labels)| {
            WorkItemUpdate {
                name,
                state_id: state.map(state_id),
                priority: Priority::ALL[priority],
                start_date: start.map(day),
                target_date: start.zip(extra).map(|(s, e)| day(s + e)).or(extra.map(day)),
                completed_at: completed.map(at),
                updated_at: at(updated),
                updated_by: uid(by),
                description_html: html,
                assignee_ids: people.map(|p| p.into_iter().map(uid).collect()),
                label_ids: labels.map(|l| l.into_iter().map(label_id).collect()),
                external_source: None,
                external_id: None,
                parent_id: None,
                point: None,
            }
        })
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// An update writes the name and every attribute it carries and who made it; the people and the labels it names replace those the
    /// item had, once each in id order, and those it does not name stay. The row is read back whole, so what it did not touch (the
    /// number, the place, the slice's description, the creation) is there as it was.
    #[test]
    fn prop_an_update_writes_what_it_names_and_keeps_the_rest(u in any_update()) {
        let db = world();
        let id = made(&db, |_| {});
        let was = row_of(&db, id).expect("the item is there");
        prop_assert_eq!(update(&db, &scope(A, P), id, &u).unwrap(), true);
        prop_assert_eq!(row_of(&db, id), Some(after(&was, &u)));
        // the page reads the same row, so the order it keeps follows the update too
        let page = block_on(db.work_item_page(&scope(A, P), &WorkItemQuery { order: WorkItemOrder { field: WorkItemOrderField::UpdatedAt, descending: true }, limit: 10, page: 0, updated_after: None, limited_guest: None })).unwrap();
        prop_assert_eq!(page.rows, vec![after(&was, &u)]);
        // the item as the list reads it carries the name
        let listed = block_on(db.list_work_items(&scope(A, P), Visibility::Visible)).unwrap();
        prop_assert_eq!(listed.iter().map(|w| w.name.clone()).collect::<Vec<_>>(), vec![u.name.clone()]);
    }

    /// The updates of one transaction apply in order, the later over the earlier, and a later one that names no people or labels
    /// leaves those the earlier one wrote.
    #[test]
    fn prop_the_updates_of_a_transaction_apply_in_order(first in any_update(), second in any_update()) {
        let db = world();
        let id = made(&db, |_| {});
        let was = row_of(&db, id).expect("the item is there");
        let s = scope(A, P);
        let mut tx = block_on(db.begin(&s)).unwrap();
        prop_assert_eq!(block_on(tx.update_work_item(&s, id, &first)).unwrap(), true);
        prop_assert_eq!(block_on(tx.update_work_item(&s, id, &second)).unwrap(), true);
        block_on(tx.commit()).unwrap();
        prop_assert_eq!(row_of(&db, id), Some(after(&after(&was, &first), &second)));
    }

    /// An item the transaction inserted can be updated in it; what is read afterwards is the item with its attributes and then the update.
    #[test]
    fn prop_an_item_inserted_in_the_transaction_can_be_updated_in_it(u in any_update()) {
        let db = world();
        let s = scope(A, P);
        let mut tx = block_on(db.begin(&s)).unwrap();
        let a = block_on(tx.allocate(&s)).unwrap();
        let item = WorkItem {
            id: WorkItemId(Uuid::from_u128(0x7777)),
            workspace_id: wid(A),
            project_id: pid(P),
            sequence_id: a.sequence_id,
            sort_order: a.sort_order,
            name: "inserted".into(),
            description: None,
            external_source: None,
            external_id: None,
            created_by: Some(uid(1)),
            created_at: at(0),
            archived_at: None,
            is_draft: false,
            deleted_at: None,
        };
        block_on(tx.insert_work_item(&s, &item)).unwrap();
        block_on(tx.insert_work_item_attrs(&s, item.id, &before())).unwrap();
        prop_assert_eq!(block_on(tx.update_work_item(&s, item.id, &u)).unwrap(), true);
        block_on(tx.commit()).unwrap();
        let was = WorkItemRow { item, attrs: before(), state_group: Some(StateGroup::Backlog) };
        prop_assert_eq!(row_of(&db, was.item.id), Some(after(&was, &u)));
    }
}

/// What the constraints refuse.
#[derive(Debug, Clone, Copy)]
enum Refusal {
    UnknownState,
    UnknownEditor,
    UnknownPerson,
    UnknownLabel,
    StartAfterTarget,
}

/// An update the constraints refuse is refused (`Backend`) and writes nothing: not its columns, not its people, not its labels; a
/// transaction that goes on to commit after the refusal leaves the item as it was, and so does one that rolls back.
#[rstest]
fn an_update_that_breaks_a_constraint_is_refused_and_writes_nothing(
    #[values(
        Refusal::UnknownState,
        Refusal::UnknownEditor,
        Refusal::UnknownPerson,
        Refusal::UnknownLabel,
        Refusal::StartAfterTarget
    )]
    refusal: Refusal,
    #[values(true, false)] commits: bool,
) {
    let db = world();
    let id = made(&db, |_| {});
    let was = row_of(&db, id).expect("the item is there");
    let mut u = change();
    match refusal {
        Refusal::UnknownState => u.state_id = Some(Uuid::from_u128(0xDEAD)),
        Refusal::UnknownEditor => u.updated_by = uid(99),
        Refusal::UnknownPerson => u.assignee_ids = Some(vec![uid(3), uid(99)]),
        Refusal::UnknownLabel => u.label_ids = Some(vec![label_id(3), Uuid::from_u128(0xDEAD)]),
        Refusal::StartAfterTarget => (u.start_date, u.target_date) = (Some(day(3)), Some(day(2))),
    }
    let s = scope(A, P);
    let mut tx = block_on(db.begin(&s)).unwrap();
    let got = block_on(tx.update_work_item(&s, id, &u));
    assert!(matches!(got, Err(DbError::Backend(_))), "{refusal:?}: {got:?}");
    if commits {
        block_on(tx.commit()).unwrap();
    } else {
        block_on(tx.rollback()).unwrap();
    }
    assert_eq!(row_of(&db, id), Some(was));
}

/// A state, a person or a label that exists but is not the project's passes (the constraints are foreign keys; the core asks first),
/// and a start on the target date is allowed.
#[test]
fn the_constraints_are_the_foreign_keys_and_the_order_of_the_dates() {
    let db = world();
    let id = made(&db, |_| {});
    let mut u = change();
    u.state_id = Some(Uuid::from_u128(ELSEWHERE));
    u.assignee_ids = Some(vec![uid(5)]);
    u.label_ids = Some(vec![Uuid::from_u128(0x8FF)]);
    (u.start_date, u.target_date) = (Some(day(4)), Some(day(4)));
    assert!(update(&db, &scope(A, P), id, &u).unwrap());
    let row = row_of(&db, id).expect("there");
    assert_eq!((row.attrs.state_id, row.attrs.label_ids), (u.state_id, vec![Uuid::from_u128(0x8FF)]));
    assert_eq!((row.attrs.start_date, row.attrs.target_date), (Some(day(4)), Some(day(4))));
}

/// Why an item cannot be updated.
#[derive(Debug, Clone, Copy)]
enum Missing {
    Unknown,
    Deleted,
    Archived,
    Draft,
    /// A live item of P, asked for through Q's scope.
    AnotherProject,
    /// A live item of P, asked for through workspace B's scope (and P).
    AnotherWorkspace,
    /// A live item of P, asked for through workspace B's scope and its project.
    AnotherTenant,
}

/// An item that is not there for the scope (unknown, deleted, archived, a draft, of another project or workspace) is `false`, and the
/// payload is not looked at (one that the constraints would refuse is `false` too), and nothing of it is written.
#[rstest]
fn an_item_that_is_not_there_is_false_and_nothing_is_written(
    #[values(
        Missing::Unknown,
        Missing::Deleted,
        Missing::Archived,
        Missing::Draft,
        Missing::AnotherProject,
        Missing::AnotherWorkspace,
        Missing::AnotherTenant
    )]
    missing: Missing,
    #[values(true, false)] refusable: bool,
) {
    let db = world();
    let live = made(&db, |_| {});
    let hidden = made(&db, |w| match missing {
        Missing::Archived => w.archived_at = Some(at(1)),
        Missing::Draft => w.is_draft = true,
        _ => {}
    });
    if matches!(missing, Missing::Deleted) {
        let s = scope(A, P);
        let mut tx = block_on(db.begin(&s)).unwrap();
        assert!(block_on(tx.soft_delete_work_item(&s, hidden, at(2))).unwrap());
        block_on(tx.commit()).unwrap();
    }
    let (id, s) = match missing {
        Missing::Unknown => (WorkItemId(Uuid::from_u128(0xBAD)), scope(A, P)),
        Missing::Deleted | Missing::Archived | Missing::Draft => (hidden, scope(A, P)),
        Missing::AnotherProject => (live, scope(A, Q)),
        Missing::AnotherWorkspace => (live, scope(B, P)),
        Missing::AnotherTenant => (live, scope(B, R)),
    };
    let mut u = change();
    if refusable {
        u.state_id = Some(Uuid::from_u128(0xDEAD));
        u.updated_by = uid(99);
    }
    let was = row_of(&db, live);
    let got = update(&db, &s, id, &u);
    assert!(matches!(got, Ok(false)), "{missing:?}: {got:?}");
    assert_eq!(row_of(&db, live), was, "{missing:?}: a live item moved");
    // the item that was asked for is as it was: still hidden, still named as it was
    let listed = block_on(db.list_work_items(&scope(A, P), Visibility::WithDeleted)).unwrap();
    assert!(listed.iter().all(|w| w.name != u.name), "{missing:?}: the name was written");
}

/// What becomes of an update when its transaction ends.
#[derive(Debug, Clone, Copy)]
enum End {
    Commit,
    Rollback,
    Drop,
}

/// An update is read after its transaction commits and not before; one rolled back or dropped is never read.
#[rstest]
fn an_update_is_seen_once_the_transaction_commits_and_never_if_it_does_not(
    #[values(End::Commit, End::Rollback, End::Drop)] end: End,
) {
    let db = world();
    let id = made(&db, |_| {});
    let was = row_of(&db, id).expect("there");
    let s = scope(A, P);
    let mut tx = block_on(db.begin(&s)).unwrap();
    assert!(block_on(tx.update_work_item(&s, id, &change())).unwrap());
    assert_eq!(row_of(&db, id), Some(was.clone()), "seen before the commit");
    match end {
        End::Commit => block_on(tx.commit()).unwrap(),
        End::Rollback => block_on(tx.rollback()).unwrap(),
        End::Drop => drop(tx),
    }
    match end {
        End::Commit => assert_eq!(row_of(&db, id), Some(after(&was, &change()))),
        _ => assert_eq!(row_of(&db, id), Some(was)),
    }
}

/// People and labels are replaced only when named, and naming none replaces them with none: the four ways of naming (nothing, an
/// empty list, a list, a list with repeats and a person who does not show) each leave what the spec says.
#[rstest]
fn people_and_labels_are_replaced_when_they_are_named_and_only_then(
    #[values(None, Some(vec![]), Some(vec![3, 3, 1]), Some(vec![4, 5, 3]))] people: Option<Vec<u128>>,
    #[values(None, Some(vec![]), Some(vec![3, 1, 3]))] labels: Option<Vec<u128>>,
) {
    let db = world();
    let id = made(&db, |_| {});
    let was = row_of(&db, id).expect("there");
    assert_eq!(was.attrs.assignee_ids, vec![uid(1), uid(2)]);
    let mut u = change();
    u.assignee_ids = people.clone().map(|p| p.into_iter().map(uid).collect());
    u.label_ids = labels.clone().map(|l| l.into_iter().map(label_id).collect());
    assert!(update(&db, &scope(A, P), id, &u).unwrap());
    let row = row_of(&db, id).expect("there");
    assert_eq!(
        row.attrs.assignee_ids,
        match &people {
            None => vec![uid(1), uid(2)],
            Some(p) => shown(&p.iter().copied().map(uid).collect::<Vec<_>>()),
        }
    );
    assert_eq!(
        row.attrs.label_ids,
        match &labels {
            None => vec![label_id(1), label_id(2)],
            Some(l) => {
                let mut v: Vec<Uuid> = l.iter().copied().map(label_id).collect();
                v.sort();
                v.dedup();
                v
            }
        }
    );
}

/// An item with no attributes (the slice's) can be updated: the update gives it all it names, and what it does not name (people,
/// labels) stays what a bare item has, none.
#[test]
fn an_item_the_slice_made_can_be_updated() {
    let db = world();
    let id = make_item(&db, A, P, |_| {});
    let mut u = change();
    (u.assignee_ids, u.label_ids) = (None, None);
    assert!(update(&db, &scope(A, P), id, &u).unwrap());
    let row = row_of(&db, id).expect("there");
    assert_eq!(
        (row.item.name.as_str(), row.attrs.priority, row.attrs.updated_by),
        ("renamed", Priority::Urgent, Some(uid(2)))
    );
    assert_eq!((row.attrs.assignee_ids, row.attrs.label_ids), (vec![], vec![]));
    assert_eq!(row.attrs.updated_at, at(50));
}

/// An item that the transaction has deleted is no longer there to update, as Postgres sees its own writes (`false`, and what the
/// transaction commits of the delete is all it commits of the item); an item it has not deleted is.
#[test]
fn an_item_this_transaction_deleted_is_not_there_to_update() {
    let db = world();
    let (gone, kept) = (made(&db, |_| {}), made(&db, |_| {}));
    let s = scope(A, P);
    let mut tx = block_on(db.begin(&s)).unwrap();
    assert!(block_on(tx.soft_delete_work_item(&s, gone, at(5))).unwrap());
    assert!(!block_on(tx.update_work_item(&s, gone, &change())).unwrap());
    assert!(block_on(tx.update_work_item(&s, kept, &change())).unwrap());
    block_on(tx.commit()).unwrap();
    assert!(row_of(&db, gone).is_none());
    assert_eq!(row_of(&db, kept).expect("there").item.name, "renamed");
    let record = block_on(db.get_work_item(&s, gone, Visibility::WithDeleted)).unwrap().expect("kept for good");
    assert_eq!(
        (record.name.as_str(), record.deleted_at),
        ("before", Some(at(5))),
        "the update did not reach the deleted item"
    );
}
