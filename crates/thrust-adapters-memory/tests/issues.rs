//! Work items with their compat attributes in the memory adapter (issue #44, migration 0013): a work item's state, priority, dates,
//! description, people and labels written in the transaction that inserts it, and a page of them read in an order. The Postgres
//! adapter is held to the same answers by the generated sequences in `thrust-adapters-selfhosted/tests/model`.
//!
//! The rules, written out:
//!
//! * `insert_work_item_attrs` belongs to the transaction of an item it inserted for the scope's project: attributes for any other
//!   item, a state, a person or a label that does not exist, and a start date after the target date refuse all of it, and a
//!   rollback leaves no attributes. The people and the labels are kept once each, in id order.
//! * A work item with no attributes (the slice's) reads as the pin's defaults: no state, priority `none`, no dates, `<p></p>`.
//! * A person assigned to an item shows only while they are an active member of the workspace and in the project.
//! * A page holds the live items of one project (not deleted, archived or a draft), `limit` of them from page number `page`,
//!   says whether another page follows and how many items match. The order is the one `WorkItemOrder` documents: a null date
//!   last, ties newest first, `priority` from `none` to `urgent` when ascending, `state_group` always from `backlog` on.

mod common;

use common::proptest_config;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::chrono::{DateTime, NaiveDate, TimeDelta};
use thrust_contracts::*;
use thrust_ports::*;

const A: u128 = 0xA;
const P: u128 = 0x10;
const OTHER: u128 = 0x11;

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

fn scope(project: u128, user: u128) -> Scope {
    Scope::mint(wid(A), pid(project), uid(user))
}

/// The four states of the project, one for each group but two (a state of `triage` and one of `cancelled` are not used here).
const STATES: [(u128, &str, StateGroup); 4] = [
    (0x501, "Backlog", StateGroup::Backlog),
    (0x502, "Todo", StateGroup::Unstarted),
    (0x503, "Doing", StateGroup::Started),
    (0x504, "Done", StateGroup::Completed),
];

fn state_id(i: usize) -> Uuid {
    Uuid::from_u128(STATES[i].0)
}

fn label_id(n: u128) -> Uuid {
    Uuid::from_u128(0x700 + n)
}

/// Workspace A with users 1 to 5 (5 is in no workspace), the project `P` (user 1 its admin, 2 a member, 3 a guest; 4 is in the
/// workspace and not in the project) with its four states and three labels, and another project.
fn world() -> MemoryDatabase {
    let db = MemoryDatabase::new();
    let state = |project: u128, n: u128, name: &str, group: StateGroup, sequence: f64, is_default: bool| SeedState {
        id: Uuid::from_u128(n + (project << 8)),
        project_id: pid(project),
        workspace_id: wid(A),
        name: name.into(),
        color: "#000000".into(),
        sequence,
        group,
        is_default,
    };
    let mut states: Vec<SeedState> = STATES
        .iter()
        .enumerate()
        .map(|(i, (id, name, group))| SeedState {
            id: Uuid::from_u128(*id),
            project_id: pid(P),
            workspace_id: wid(A),
            name: (*name).into(),
            color: "#000000".into(),
            sequence: 10_000.0 * (i as f64 + 1.0),
            group: *group,
            is_default: i == 0,
        })
        .collect();
    states.push(state(OTHER, 1, "Elsewhere", StateGroup::Backlog, 1.0, true));
    let label = |project: u128, n: u128| SeedLabel {
        id: if project == P { label_id(n) } else { Uuid::from_u128(0x800 + n) },
        project_id: pid(project),
        workspace_id: wid(A),
        name: format!("l{project}-{n}"),
        color: "#fff".into(),
        parent_id: None,
        sort_order: n as f64,
    };
    let f = SeedFixture {
        workspaces: vec![SeedWorkspace { id: wid(A), slug: WorkspaceSlug::new("a") }],
        users: (1..=5).map(uid).collect(),
        workspace_members: (1..=4).map(|u| SeedWorkspaceMember { workspace_id: wid(A), user_id: uid(u) }).collect(),
        projects: vec![
            SeedProject { id: pid(P), workspace_id: wid(A), name: "p".into() },
            SeedProject { id: pid(OTHER), workspace_id: wid(A), name: "other".into() },
        ],
        memberships: [(1, Role::Admin), (2, Role::Member), (3, Role::Guest)]
            .iter()
            .map(|(u, role)| SeedMembership { project_id: pid(P), workspace_id: wid(A), user_id: uid(*u), role: *role })
            .collect(),
        states,
        labels: vec![label(P, 1), label(P, 2), label(P, 3), label(OTHER, 1)],
        ..SeedFixture::default()
    };
    block_on(db.seed(&sys(), &f)).unwrap();
    db
}

/// A work item as the tests make it: what makes its place in an order.
#[derive(Debug, Clone)]
struct Spec {
    priority: Priority,
    state: Option<usize>,
    start: Option<i64>,
    target: Option<i64>,
    completed: Option<i64>,
    created: i64,
    updated: i64,
}

impl Spec {
    fn plain() -> Spec {
        Spec { priority: Priority::None, state: None, start: None, target: None, completed: None, created: 0, updated: 0 }
    }

    fn attrs(&self, by: u128) -> WorkItemAttrs {
        WorkItemAttrs {
            state_id: self.state.map(state_id),
            priority: self.priority,
            start_date: self.start.map(day),
            target_date: self.target.map(day),
            completed_at: self.completed.map(at),
            updated_at: at(self.updated),
            updated_by: Some(uid(by)),
            description_html: "<p>x</p>".into(),
            assignee_ids: vec![],
            label_ids: vec![],
        }
    }
}

/// Insert one work item of `project` with `spec`'s attributes, in a transaction of its own; its sequence number.
fn make(db: &MemoryDatabase, project: u128, spec: &Spec, attrs: Option<WorkItemAttrs>) -> (WorkItemId, i64) {
    let s = scope(project, 1);
    let mut tx = block_on(db.begin(&s)).unwrap();
    let a = block_on(tx.allocate(&s)).unwrap();
    let id = WorkItemId(Uuid::from_u128(0x9000 + (project << 16) + a.sequence_id as u128));
    let item = WorkItem {
        id,
        workspace_id: wid(A),
        project_id: pid(project),
        sequence_id: a.sequence_id,
        sort_order: a.sort_order,
        name: format!("item {}", a.sequence_id),
        description: None,
        external_source: None,
        external_id: None,
        created_by: uid(1),
        created_at: at(spec.created),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    block_on(tx.insert_work_item(&s, &item)).unwrap();
    block_on(tx.insert_work_item_attrs(&s, id, &attrs.unwrap_or_else(|| spec.attrs(1)))).unwrap();
    block_on(tx.commit()).unwrap();
    (id, a.sequence_id)
}

fn query(order: WorkItemOrder, limit: u32, page: u32) -> WorkItemQuery {
    WorkItemQuery { order, limit, page, updated_after: None, created_by: None }
}

fn newest_first() -> WorkItemOrder {
    WorkItemOrder { field: WorkItemOrderField::CreatedAt, descending: true }
}

// --- attributes ---------------------------------------------------------------------------------------------------------------

/// What a work item the slice made reads as: the pin's defaults, from the time it was made.
#[test]
fn a_work_item_with_no_attributes_reads_as_the_pins_defaults() {
    let db = world();
    let s = scope(P, 1);
    let mut tx = block_on(db.begin(&s)).unwrap();
    let a = block_on(tx.allocate(&s)).unwrap();
    let item = WorkItem {
        id: WorkItemId(Uuid::from_u128(0x77)),
        workspace_id: wid(A),
        project_id: pid(P),
        sequence_id: a.sequence_id,
        sort_order: a.sort_order,
        name: "made by the slice".into(),
        description: Some("text".into()),
        external_source: None,
        external_id: None,
        created_by: uid(1),
        created_at: at(5),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    block_on(tx.insert_work_item(&s, &item)).unwrap();
    block_on(tx.commit()).unwrap();
    let row = block_on(db.work_item_row(&s, item.id)).unwrap().expect("the item is there");
    assert_eq!(row.attrs, WorkItemAttrs::bare(at(5)));
    assert_eq!((row.item, row.state_group), (item, None));
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// Whatever attributes an item is given come back as they were, with the people and the labels once each and in id order, and
    /// the group of its state; the page reads the same row.
    #[test]
    fn prop_attributes_come_back_as_they_were_given(
        priority in 0..5usize,
        state in prop::option::of(0..4usize),
        start in prop::option::of(0..6i64),
        extra in prop::option::of(0..4i64),
        people in prop::collection::vec(1..=3u128, 0..6),
        labels in prop::collection::vec(1..=3u128, 0..6),
        html in "[ -~]{0,20}",
    ) {
        let db = world();
        let spec = Spec { priority: Priority::ALL[priority], state, start, target: start.zip(extra).map(|(s, e)| s + e).or(extra), ..Spec::plain() };
        let mut attrs = spec.attrs(2);
        attrs.description_html = html;
        attrs.assignee_ids = people.iter().map(|u| uid(*u)).collect();
        attrs.label_ids = labels.iter().map(|l| label_id(*l)).collect();
        let (id, _) = make(&db, P, &spec, Some(attrs.clone()));
        let mut want = attrs;
        want.assignee_ids.sort();
        want.assignee_ids.dedup();
        want.label_ids.sort();
        want.label_ids.dedup();
        let row = block_on(db.work_item_row(&scope(P, 1), id)).unwrap().expect("there");
        prop_assert_eq!(&row.attrs, &want);
        prop_assert_eq!(row.state_group, state.map(|i| STATES[i].2));
        let page = block_on(db.work_item_page(&scope(P, 1), &query(newest_first(), 10, 0))).unwrap();
        prop_assert_eq!(page.rows, vec![row]);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    UnknownState,
    UnknownPerson,
    UnknownLabel,
    StartAfterTarget,
}

/// Attributes that Postgres's constraints refuse are refused here (`Backend`), all of them: the item that was inserted in the same
/// transaction is not there afterwards either.
#[rstest]
fn attributes_that_break_a_constraint_are_refused_and_the_transaction_writes_nothing(
    #[values(Refusal::UnknownState, Refusal::UnknownPerson, Refusal::UnknownLabel, Refusal::StartAfterTarget)] refusal: Refusal,
) {
    let db = world();
    let s = scope(P, 1);
    let mut attrs = Spec::plain().attrs(1);
    match refusal {
        Refusal::UnknownState => attrs.state_id = Some(Uuid::from_u128(0xDEAD)),
        Refusal::UnknownPerson => attrs.assignee_ids = vec![uid(1), uid(99)],
        Refusal::UnknownLabel => attrs.label_ids = vec![label_id(1), Uuid::from_u128(0xDEAD)],
        Refusal::StartAfterTarget => (attrs.start_date, attrs.target_date) = (Some(day(3)), Some(day(2))),
    }
    let mut tx = block_on(db.begin(&s)).unwrap();
    let a = block_on(tx.allocate(&s)).unwrap();
    let item = WorkItem {
        id: WorkItemId(Uuid::from_u128(0x78)),
        workspace_id: wid(A),
        project_id: pid(P),
        sequence_id: a.sequence_id,
        sort_order: a.sort_order,
        name: "n".into(),
        description: None,
        external_source: None,
        external_id: None,
        created_by: uid(1),
        created_at: at(0),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    block_on(tx.insert_work_item(&s, &item)).unwrap();
    let got = block_on(tx.insert_work_item_attrs(&s, item.id, &attrs));
    assert!(matches!(got, Err(DbError::Backend(_))), "{refusal:?}: {got:?}");
    block_on(tx.rollback()).unwrap();
    assert!(block_on(db.work_item_row(&s, item.id)).unwrap().is_none());
    assert_eq!(block_on(db.work_item_page(&s, &query(newest_first(), 10, 0))).unwrap().total, 0);
}

/// Attributes are for an item this transaction inserted, for the scope's project: not for one that is not there, one of another
/// transaction, or one of another project.
#[test]
fn attributes_are_for_an_item_the_transaction_inserted_in_the_scopes_project() {
    let db = world();
    let (earlier, _) = make(&db, P, &Spec::plain(), None);
    let s = scope(P, 1);
    let mut tx = block_on(db.begin(&s)).unwrap();
    for id in [earlier, WorkItemId(Uuid::from_u128(0xBAD))] {
        let got = block_on(tx.insert_work_item_attrs(&s, id, &Spec::plain().attrs(1)));
        assert!(matches!(got, Err(DbError::Backend(_))), "{id:?}: {got:?}");
    }
    // an item of the other project, inserted here, is not the scope's
    let a = block_on(tx.allocate(&scope(OTHER, 1))).unwrap();
    let elsewhere = WorkItem {
        id: WorkItemId(Uuid::from_u128(0x79)),
        workspace_id: wid(A),
        project_id: pid(OTHER),
        sequence_id: a.sequence_id,
        sort_order: a.sort_order,
        name: "n".into(),
        description: None,
        external_source: None,
        external_id: None,
        created_by: uid(1),
        created_at: at(0),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    block_on(tx.insert_work_item(&scope(OTHER, 1), &elsewhere)).unwrap();
    assert!(matches!(block_on(tx.insert_work_item_attrs(&s, elsewhere.id, &Spec::plain().attrs(1))), Err(DbError::Backend(_))));
}

/// A person assigned to an item shows while they are an active member of the workspace and in the project, and not when either
/// stops being so; the row stays, so they show again when they return.
#[test]
fn an_assignee_shows_only_while_an_active_member_of_the_workspace_and_in_the_project() {
    let db = world();
    let mut attrs = Spec::plain().attrs(1);
    // 2 is in the project; 4 is in the workspace and not in the project; 3 is a guest of it; 1 is its admin
    attrs.assignee_ids = vec![uid(1), uid(2), uid(3), uid(4)];
    let (id, _) = make(&db, P, &Spec::plain(), Some(attrs));
    let shown = || block_on(db.work_item_row(&scope(P, 1), id)).unwrap().unwrap().attrs.assignee_ids;
    assert_eq!(shown(), vec![uid(1), uid(2), uid(3)], "4 is in no project: not shown");
    assert!(block_on(db.set_workspace_member_active(&sys(), wid(A), uid(2), false)).unwrap());
    assert_eq!(shown(), vec![uid(1), uid(3)], "2 has left the workspace");
    assert!(block_on(db.set_workspace_member_active(&sys(), wid(A), uid(2), true)).unwrap());
    assert_eq!(shown(), vec![uid(1), uid(2), uid(3)], "and is back");
}

// --- a page -------------------------------------------------------------------------------------------------------------------

/// A work item as the oracle sees it.
#[derive(Debug, Clone)]
struct Made {
    seq: i64,
    spec: Spec,
}

fn state_name(m: &Made) -> Option<&'static str> {
    m.spec.state.map(|i| STATES[i].1)
}

fn group_rank(m: &Made) -> u8 {
    match m.spec.state.map(|i| STATES[i].2) {
        Some(StateGroup::Backlog) => 0,
        Some(StateGroup::Unstarted) => 1,
        Some(StateGroup::Started) => 2,
        Some(StateGroup::Completed) => 3,
        _ => 5,
    }
}

/// The order the port documents, written as a stable sort on top of the newest first order (so that a tie is left newest first):
/// the oracle for the adapter's comparator, which is written as one comparison.
fn oracle(items: &[Made], field: WorkItemOrderField, descending: bool) -> Vec<i64> {
    let mut v: Vec<Made> = items.to_vec();
    v.sort_by(|a, b| (b.spec.created, b.seq).cmp(&(a.spec.created, a.seq)));
    // dates and times: the ones that are null go after the others, whichever way it runs
    fn nulls_last<K: Ord + Copy>(v: &mut [Made], key: impl Fn(&Made) -> Option<K>, descending: bool) {
        v.sort_by(|a, b| match (key(a), key(b)) {
            (Some(x), Some(y)) => if descending { y.cmp(&x) } else { x.cmp(&y) },
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
    }
    match field {
        // these two break their own ties the way they run
        WorkItemOrderField::CreatedAt => v.sort_by_key(|m| (m.spec.created, m.seq)),
        WorkItemOrderField::UpdatedAt => v.sort_by_key(|m| (m.spec.updated, m.seq)),
        WorkItemOrderField::SequenceId => v.sort_by_key(|m| m.seq),
        // a project's items are numbered and given a place in one step each, so the two run together
        WorkItemOrderField::SortOrder => v.sort_by_key(|m| m.seq),
        WorkItemOrderField::TargetDate => nulls_last(&mut v, |m| m.spec.target, descending),
        WorkItemOrderField::StartDate => nulls_last(&mut v, |m| m.spec.start, descending),
        WorkItemOrderField::CompletedAt => nulls_last(&mut v, |m| m.spec.completed, descending),
        WorkItemOrderField::ArchivedAt => {}
        WorkItemOrderField::Priority => {
            // descending runs from urgent (rank 0); ascending from none (rank 4)
            v.sort_by_key(|m| if descending { i32::from(m.spec.priority.rank()) } else { -i32::from(m.spec.priority.rank()) });
        }
        WorkItemOrderField::StateName => nulls_last(&mut v, |m| state_name(m), descending),
        WorkItemOrderField::StateGroup => v.sort_by_key(group_rank),
    }
    let by_key_only = matches!(field, WorkItemOrderField::CreatedAt | WorkItemOrderField::UpdatedAt | WorkItemOrderField::SequenceId | WorkItemOrderField::SortOrder);
    let mut seqs: Vec<i64> = v.iter().map(|m| m.seq).collect();
    if by_key_only && descending {
        seqs.reverse();
    }
    seqs
}

fn spec_strategy() -> impl Strategy<Value = Spec> {
    (
        0..5usize,
        prop::option::of(0..4usize),
        prop::option::of(0..5i64),
        prop::option::of(0..3i64),
        prop::option::of(0..3i64),
        0..3i64,
        0..3i64,
    )
        .prop_map(|(priority, state, start, extra, completed, created, updated)| Spec {
            priority: Priority::ALL[priority],
            state,
            start,
            target: start.zip(extra).map(|(s, e)| s + e).or_else(|| if start.is_none() { extra } else { None }),
            completed,
            created,
            updated,
        })
}

proptest! {
    #![proptest_config(proptest_config(96))]

    /// A page is a slice of the order the port documents, in any order and any page size: the pages one after another are the
    /// whole list, each says whether another follows, and every one says how many items there are.
    #[test]
    fn prop_pages_are_slices_of_the_documented_order(
        specs in prop::collection::vec(spec_strategy(), 0..14),
        field in 0..11usize,
        descending in any::<bool>(),
        limit in 1..6u32,
    ) {
        let db = world();
        let made: Vec<Made> = specs.iter().map(|s| Made { seq: make(&db, P, s, None).1, spec: s.clone() }).collect();
        let order = WorkItemOrder { field: WorkItemOrderField::ALL[field], descending };
        let want = oracle(&made, order.field, order.descending);
        let mut got = Vec::new();
        for page in 0.. {
            let p = block_on(db.work_item_page(&scope(P, 1), &query(order, limit, page))).unwrap();
            prop_assert_eq!(p.total, made.len() as u64);
            prop_assert!(p.rows.len() <= limit as usize);
            got.extend(p.rows.iter().map(|r| r.item.sequence_id));
            prop_assert_eq!(p.has_next, got.len() < want.len(), "page {}", page);
            if !p.has_next {
                break;
            }
        }
        prop_assert_eq!(got, want, "{:?}", order);
        // a page past the end is empty, and still says how many there are
        let past = (made.len() as u32 / limit) + 1;
        let p = block_on(db.work_item_page(&scope(P, 1), &query(order, limit, past))).unwrap();
        prop_assert_eq!((p.rows.len(), p.has_next, p.total), (0, false, made.len() as u64));
    }
}

/// A page and a row hold the live items of the project: not another project's, not a deleted, archived or draft one.
#[test]
fn a_page_holds_only_the_live_items_of_the_project() {
    let db = world();
    let (live, _) = make(&db, P, &Spec::plain(), None);
    let (gone, _) = make(&db, P, &Spec::plain(), None);
    let (elsewhere, _) = make(&db, OTHER, &Spec::plain(), None);
    let s = scope(P, 1);
    let mut tx = block_on(db.begin(&s)).unwrap();
    assert!(block_on(tx.soft_delete_work_item(&s, gone, at(9))).unwrap());
    block_on(tx.commit()).unwrap();
    // an archived item and a draft: what the slice has no route to make, so each is inserted whole
    for (n, is_draft) in [(0x2000u128, false), (0x2001, true)] {
        let mut tx = block_on(db.begin(&s)).unwrap();
        let a = block_on(tx.allocate(&s)).unwrap();
        let hidden = WorkItem {
            id: WorkItemId(Uuid::from_u128(n)),
            workspace_id: wid(A),
            project_id: pid(P),
            sequence_id: a.sequence_id,
            sort_order: a.sort_order,
            name: "hidden".into(),
            description: None,
            external_source: None,
            external_id: None,
            created_by: uid(1),
            created_at: at(0),
            archived_at: (!is_draft).then(|| at(9)),
            is_draft,
            deleted_at: None,
        };
        block_on(tx.insert_work_item(&s, &hidden)).unwrap();
        block_on(tx.commit()).unwrap();
    }
    let page = block_on(db.work_item_page(&s, &query(newest_first(), 10, 0))).unwrap();
    assert_eq!(page.rows.iter().map(|r| r.item.id).collect::<Vec<_>>(), vec![live]);
    assert_eq!(page.total, 1);
    assert!(block_on(db.work_item_row(&s, gone)).unwrap().is_none());
    assert!(block_on(db.work_item_row(&s, elsewhere)).unwrap().is_none(), "another project's item is not this project's");
    assert!(block_on(db.work_item_row(&scope(OTHER, 1), elsewhere)).unwrap().is_some());
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// `updated_after` leaves out what was changed at or before it, and `created_by` keeps what one person made; both count in the
    /// total, and both can be asked together.
    #[test]
    fn prop_a_page_can_be_cut_by_when_it_changed_and_by_who_made_it(
        specs in prop::collection::vec((spec_strategy(), 1..=3u128), 0..12),
        after in prop::option::of(0..3i64),
        by in prop::option::of(1..=3u128),
    ) {
        let db = world();
        let mut kept = 0u64;
        for (spec, creator) in &specs {
            let s = scope(P, *creator);
            let mut tx = block_on(db.begin(&s)).unwrap();
            let a = block_on(tx.allocate(&s)).unwrap();
            let id = WorkItemId(Uuid::from_u128(0x9000 + a.sequence_id as u128));
            let item = WorkItem {
                id,
                workspace_id: wid(A),
                project_id: pid(P),
                sequence_id: a.sequence_id,
                sort_order: a.sort_order,
                name: "n".into(),
                description: None,
                external_source: None,
                external_id: None,
                created_by: uid(*creator),
                created_at: at(spec.created),
                archived_at: None,
                is_draft: false,
                deleted_at: None,
            };
            block_on(tx.insert_work_item(&s, &item)).unwrap();
            block_on(tx.insert_work_item_attrs(&s, id, &spec.attrs(*creator))).unwrap();
            block_on(tx.commit()).unwrap();
            if after.map_or(true, |t| spec.updated > t) && by.map_or(true, |u| u == *creator) {
                kept += 1;
            }
        }
        let q = WorkItemQuery { order: newest_first(), limit: 100, page: 0, updated_after: after.map(at), created_by: by.map(uid) };
        let p = block_on(db.work_item_page(&scope(P, 1), &q)).unwrap();
        prop_assert_eq!((p.rows.len() as u64, p.total), (kept, kept));
    }
}

/// An item as `prop_the_greatest_place_...` makes it: in the other project or this one, in a state or none, live, deleted, archived
/// or a draft, at a place of its own choosing.
#[derive(Debug, Clone, Copy)]
struct Placed {
    other_project: bool,
    state: Option<usize>,
    kind: u8,
    place: i64,
}

proptest! {
    #![proptest_config(proptest_config(96))]

    /// Spec (what the pin puts a new item's place after): the greatest place among the project's items that are in the same state, or
    /// that have none; an archived item and a draft count, a deleted one and another project's do not; no such item, no place.
    #[test]
    fn prop_the_greatest_place_is_taken_among_the_projects_items_of_one_state(
        items in prop::collection::vec((any::<bool>(), prop::option::of(0..4usize), 0..4u8, 0..100_000i64), 0..14),
    ) {
        let db = world();
        // the other project has one state of its own, so its items have none: the same "no state" as this project's, to be told apart
        let placed: Vec<Placed> = items
            .iter()
            .map(|(other_project, state, kind, place)| Placed { other_project: *other_project, state: if *other_project { None } else { *state }, kind: *kind, place: *place })
            .collect();
        for p in &placed {
            let project = if p.other_project { OTHER } else { P };
            let s = scope(project, 1);
            let mut tx = block_on(db.begin(&s)).unwrap();
            let a = block_on(tx.allocate(&s)).unwrap();
            let id = WorkItemId(Uuid::from_u128(0x9000 + (project << 16) + a.sequence_id as u128));
            let item = WorkItem {
                id,
                workspace_id: wid(A),
                project_id: pid(project),
                sequence_id: a.sequence_id,
                sort_order: p.place,
                name: "n".into(),
                description: None,
                external_source: None,
                external_id: None,
                created_by: uid(1),
                created_at: at(0),
                archived_at: (p.kind == 2).then(|| at(9)),
                is_draft: p.kind == 3,
                deleted_at: None,
            };
            block_on(tx.insert_work_item(&s, &item)).unwrap();
            block_on(tx.insert_work_item_attrs(&s, id, &Spec { state: p.state, ..Spec::plain() }.attrs(1))).unwrap();
            block_on(tx.commit()).unwrap();
            if p.kind == 1 {
                let mut tx = block_on(db.begin(&s)).unwrap();
                prop_assert!(block_on(tx.soft_delete_work_item(&s, id, at(9))).unwrap());
                block_on(tx.commit()).unwrap();
            }
        }
        for probe in [None, Some(0), Some(1), Some(2), Some(3)] {
            let want = placed.iter().filter(|p| !p.other_project && p.state == probe && p.kind != 1).map(|p| p.place).max();
            let got = block_on(db.max_work_item_sort_order(&scope(P, 1), probe.map(state_id))).unwrap();
            prop_assert_eq!(got, want, "state {:?}", probe);
        }
    }
}
