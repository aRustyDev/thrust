//! What the seed does with the fixture's work items, and what `reset` leaves (issue #58). The Postgres adapter is held to the same
//! answers by `pg_adapter.rs` in `thrust-adapters-selfhosted`.
//!
//! The rules, written out:
//!
//! * A work item of the fixture is stored as given: its number, its place, its text, its time and its maker, and what a request
//!   would have given it (state, priority, dates, when it was completed, description, parent). The people and the labels it names
//!   are kept once each and in id order, as `insert_work_item_attrs` keeps them.
//! * A project's next number and place come after the largest the fixture gave: a project whose fixture has no work item numbers
//!   from 1 and places from 10000.
//! * The database refuses what its constraints refuse, from a fixture as from a request: a number the project has, a project that
//!   is not in the workspace named, a state, person, label or parent that is not there (the parent must come earlier in the
//!   fixture), a start after the target.
//! * `reset` leaves nothing: no row of any table and no counter. The same fixture loads again into what is left, and gives the
//!   same rows as it did the first time.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use common::proptest_config;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::chrono::{DateTime, Duration, NaiveDate};
use thrust_contracts::*;
use thrust_ports::*;

const WS: u128 = 0xA;
const PROJECT: u128 = 0x10;
const OTHER_PROJECT: u128 = 0x11;
const STATE: [u128; 3] = [0x5001, 0x5002, 0x5003];
const LABEL: [u128; 3] = [0x6001, 0x6002, 0x6003];

fn ws() -> WorkspaceId {
    WorkspaceId(Uuid::from_u128(WS))
}
fn project() -> ProjectId {
    ProjectId(Uuid::from_u128(PROJECT))
}
fn user(n: u128) -> UserId {
    UserId(Uuid::from_u128(n))
}
fn scope() -> Scope {
    Scope::mint(ws(), project(), user(1))
}
fn at(n: i64) -> Timestamp {
    DateTime::from_timestamp(1_760_000_000 + n, 0).unwrap()
}
fn day(n: i64) -> NaiveDate {
    at(0).date_naive() + Duration::days(n)
}

/// What a generated work item says: indices into the world's states (3), labels (3) and users (1 to 4), and the one it is under.
#[derive(Debug, Clone)]
struct Spec {
    state: Option<usize>,
    priority: usize,
    assignees: Vec<usize>,
    labels: Vec<usize>,
    parent: Option<usize>,
    start: Option<u8>,
    span: Option<u8>,
    completed: bool,
    skip: u8,
    place: u32,
    maker: Option<u8>,
}

fn arb_spec() -> impl Strategy<Value = Spec> {
    (
        prop::option::of(0..3_usize),
        0..5_usize,
        prop::collection::vec(0..4_usize, 0..5),
        prop::collection::vec(0..3_usize, 0..5),
        prop::option::of(any::<prop::sample::Index>()),
        prop::option::of(0..40_u8),
        prop::option::of(0..40_u8),
        any::<bool>(),
        0..4_u8,
        1..2_000_000_u32,
        prop::option::of(1..5_u8),
    )
        .prop_map(|(state, priority, assignees, labels, parent, start, span, completed, skip, place, maker)| {
            Spec {
                state,
                priority,
                assignees,
                labels,
                // an index into the items before it, so that a parent is made before its children
                parent: parent.map(|p| p.index(usize::MAX)),
                start,
                span,
                completed,
                skip,
                place,
                maker,
            }
        })
}

/// Up to eight items; the longer the list the more of the chains of parents are reached.
fn arb_specs() -> impl Strategy<Value = Vec<Spec>> {
    prop::collection::vec(arb_spec(), 0..8)
}

/// The fixture: a workspace with four users, a project (and another, in the same workspace, that the work items are not in), three
/// states, three labels, and the items the specs describe, numbered upward with gaps.
fn fixture(specs: &[Spec]) -> SeedFixture {
    let mut sequence = 0;
    let mut items: Vec<SeedIssue> = Vec::new();
    for (i, s) in specs.iter().enumerate() {
        sequence += 1 + i64::from(s.skip);
        let id = WorkItemId(Uuid::from_u128(0x9000 + i as u128));
        items.push(SeedIssue {
            id,
            workspace_id: ws(),
            project_id: project(),
            sequence_id: sequence,
            sort_order: i64::from(s.place),
            name: format!("item {i}"),
            description: Some(format!("text {i}")),
            description_html: format!("<p>text {i}</p>"),
            state_id: s.state.map(|n| Uuid::from_u128(STATE[n])),
            priority: Priority::ALL[s.priority],
            parent_id: s.parent.filter(|_| i > 0).map(|p| items[p % i].id.0),
            start_date: s.start.map(|d| day(i64::from(d))),
            target_date: match (s.start, s.span) {
                (Some(a), Some(b)) => Some(day(i64::from(a) + i64::from(b))),
                (None, Some(b)) => Some(day(i64::from(b))),
                _ => None,
            },
            completed_at: s.completed.then(|| at(i as i64)),
            assignee_ids: s.assignees.iter().map(|n| user(*n as u128 + 1)).collect(),
            label_ids: s.labels.iter().map(|n| Uuid::from_u128(LABEL[*n])).collect(),
            created_by: s.maker.map(|n| user(u128::from(n))),
            created_at: at(i as i64),
        });
    }
    let mut f = world();
    f.work_items = items;
    f
}

/// The world without work items.
fn world() -> SeedFixture {
    let groups = [StateGroup::Backlog, StateGroup::Unstarted, StateGroup::Completed];
    SeedFixture {
        workspaces: vec![SeedWorkspace { id: ws(), slug: WorkspaceSlug::new("a") }],
        users: (1..=4).map(user).collect(),
        workspace_roles: (1..=4)
            .map(|n| SeedWorkspaceRole { workspace_id: ws(), user_id: user(n), role: Role::Admin })
            .collect(),
        projects: vec![
            SeedProject { id: project(), workspace_id: ws(), name: "p".into() },
            SeedProject { id: ProjectId(Uuid::from_u128(OTHER_PROJECT)), workspace_id: ws(), name: "q".into() },
        ],
        memberships: (1..=4)
            .map(|n| SeedMembership { project_id: project(), workspace_id: ws(), user_id: user(n), role: Role::Admin })
            .collect(),
        states: (0..3)
            .map(|n| SeedState {
                id: Uuid::from_u128(STATE[n]),
                project_id: project(),
                workspace_id: ws(),
                name: format!("state {n}"),
                color: "#000000".into(),
                sequence: 15_000.0 * (n as f64 + 1.0),
                group: groups[n],
                is_default: n == 0,
            })
            .collect(),
        labels: (0..3)
            .map(|n| SeedLabel {
                id: Uuid::from_u128(LABEL[n]),
                project_id: project(),
                workspace_id: ws(),
                name: format!("label {n}"),
                color: "#111111".into(),
                parent_id: None,
                sort_order: 65_535.0 + 10_000.0 * n as f64,
            })
            .collect(),
        ..SeedFixture::default()
    }
}

fn sys() -> SystemScope {
    SystemScope::mint()
}

fn sorted_dedup<T: Ord + Clone>(v: &[T]) -> Vec<T> {
    let mut out = v.to_vec();
    out.sort();
    out.dedup();
    out
}

proptest! {
    #![proptest_config(proptest_config(256))]

    #[test]
    fn prop_a_seeded_work_item_reads_back_as_it_was_given(specs in arb_specs()) {
        let f = fixture(&specs);
        let db = MemoryDatabase::new();
        block_on(db.seed(&sys(), &f)).unwrap();
        for w in &f.work_items {
            let row = block_on(db.work_item_row(&scope(), w.id)).unwrap().expect("a seeded item is there");
            prop_assert_eq!(row.item.sequence_id, w.sequence_id);
            prop_assert_eq!(row.item.sort_order, w.sort_order);
            prop_assert_eq!(&row.item.name, &w.name);
            prop_assert_eq!(&row.item.description, &w.description);
            prop_assert_eq!(row.item.created_by, w.created_by);
            prop_assert_eq!(row.item.created_at, w.created_at);
            prop_assert_eq!((row.item.archived_at, row.item.is_draft, row.item.deleted_at), (None, false, None));
            prop_assert_eq!(row.attrs.state_id, w.state_id);
            prop_assert_eq!(row.attrs.priority, w.priority);
            prop_assert_eq!(row.attrs.start_date, w.start_date);
            prop_assert_eq!(row.attrs.target_date, w.target_date);
            prop_assert_eq!(row.attrs.completed_at, w.completed_at);
            prop_assert_eq!(&row.attrs.description_html, &w.description_html);
            prop_assert_eq!(row.attrs.parent_id, w.parent_id);
            prop_assert_eq!(row.attrs.point, None);
            prop_assert_eq!(&row.attrs.assignee_ids, &sorted_dedup(&w.assignee_ids));
            prop_assert_eq!(&row.attrs.label_ids, &sorted_dedup(&w.label_ids));
            // the group of its state comes from the state it names
            let group = w.state_id.and_then(|s| f.states.iter().find(|x| x.id == s)).map(|s| s.group);
            prop_assert_eq!(row.state_group, group);
        }
        // and the number finds it: the lookup by number is the one the v1 API uses for `KEY-n`
        for w in &f.work_items {
            let found = block_on(db.work_item_by_sequence(&scope(), w.sequence_id)).unwrap().expect("found by number");
            prop_assert_eq!(found.item.id, w.id);
        }
    }

    #[test]
    fn prop_the_next_number_and_place_come_after_the_seeded_ones(specs in arb_specs()) {
        let f = fixture(&specs);
        let db = MemoryDatabase::new();
        block_on(db.seed(&sys(), &f)).unwrap();
        let largest_number = f.work_items.iter().map(|w| w.sequence_id).max().unwrap_or(0);
        let largest_place = f.work_items.iter().map(|w| w.sort_order).max().unwrap_or(0);
        let mut tx = block_on(db.begin(&scope())).unwrap();
        let first = block_on(tx.allocate(&scope())).unwrap();
        prop_assert_eq!((first.sequence_id, first.sort_order), (largest_number + 1, largest_place + 10_000));
        // and it goes on from there, one by one
        let second = block_on(tx.allocate(&scope())).unwrap();
        prop_assert_eq!((second.sequence_id, second.sort_order), (largest_number + 2, largest_place + 20_000));
    }

    #[test]
    fn prop_a_reset_leaves_nothing_and_the_same_fixture_loads_again(specs in arb_specs()) {
        let f = fixture(&specs);
        let db = MemoryDatabase::new();
        block_on(db.seed(&sys(), &f)).unwrap();
        block_on(db.reset(&sys())).unwrap();
        // no row of any table that a seed fills (a work item in no state either: the deleted, archived and draft ones count)
        for w in &f.work_items {
            prop_assert!(block_on(db.get_work_item(&scope(), w.id, Visibility::WithDeleted)).unwrap().is_none());
        }
        prop_assert!(block_on(db.project_states(&scope())).unwrap().is_empty());
        prop_assert!(block_on(db.project_labels(&scope())).unwrap().is_empty());
        prop_assert!(block_on(db.user_by_id(user(1))).unwrap().is_none());
        prop_assert!(!block_on(db.workspace_slug_taken("a")).unwrap());
        for w in &f.work_items {
            prop_assert!(block_on(db.work_item_row(&scope(), w.id)).unwrap().is_none());
        }
        // no counter: the project is not there to number from
        let mut tx = block_on(db.begin(&scope())).unwrap();
        prop_assert!(matches!(block_on(tx.allocate(&scope())), Err(DbError::Backend(_))));
        drop(tx);
        // the same fixture loads without a clash, and numbers from the same place as the first time
        block_on(db.seed(&sys(), &f)).unwrap();
        let mut tx = block_on(db.begin(&scope())).unwrap();
        let first = block_on(tx.allocate(&scope())).unwrap();
        let largest_number = f.work_items.iter().map(|w| w.sequence_id).max().unwrap_or(0);
        prop_assert_eq!(first.sequence_id, largest_number + 1);
        for w in &f.work_items {
            prop_assert!(block_on(db.work_item_row(&scope(), w.id)).unwrap().is_some());
        }
    }
}

/// What a fixture can get wrong, one way at a time. Each is a corruption of a valid fixture that has two items (the second under
/// the first), and each is refused as the database refuses it.
#[derive(Debug, Clone, Copy)]
enum Wrong {
    DuplicateNumber,
    ProjectOfAnotherWorkspace,
    UnknownState,
    UnknownAssignee,
    UnknownLabel,
    ParentNotThere,
    ParentComesLater,
    StartAfterTarget,
}

#[rstest]
fn a_fixture_is_refused_as_the_database_refuses_a_request(
    #[values(
        Wrong::DuplicateNumber,
        Wrong::ProjectOfAnotherWorkspace,
        Wrong::UnknownState,
        Wrong::UnknownAssignee,
        Wrong::UnknownLabel,
        Wrong::ParentNotThere,
        Wrong::ParentComesLater,
        Wrong::StartAfterTarget
    )]
    wrong: Wrong,
) {
    // the valid fixture: item 1 under nothing, item 2 under item 1
    let plain = Spec {
        state: Some(0),
        priority: 0,
        assignees: vec![0],
        labels: vec![0],
        parent: None,
        start: None,
        span: None,
        completed: false,
        skip: 0,
        place: 100,
        maker: None,
    };
    let mut f = fixture(&[plain.clone(), Spec { parent: Some(0), ..plain }]);
    assert_eq!(f.work_items[1].parent_id, Some(f.work_items[0].id.0), "the fixture of the test is what it says");
    block_on(MemoryDatabase::new().seed(&sys(), &f)).expect("the valid fixture loads");
    let conflict = matches!(wrong, Wrong::DuplicateNumber);
    match wrong {
        Wrong::DuplicateNumber => f.work_items[1].sequence_id = f.work_items[0].sequence_id,
        Wrong::ProjectOfAnotherWorkspace => f.work_items[0].workspace_id = WorkspaceId(Uuid::from_u128(0xB)),
        Wrong::UnknownState => f.work_items[0].state_id = Some(Uuid::from_u128(0xDEAD)),
        Wrong::UnknownAssignee => f.work_items[0].assignee_ids.push(user(99)),
        Wrong::UnknownLabel => f.work_items[0].label_ids.push(Uuid::from_u128(0xDEAD)),
        Wrong::ParentNotThere => f.work_items[1].parent_id = Some(Uuid::from_u128(0xDEAD)),
        Wrong::ParentComesLater => f.work_items.reverse(),
        Wrong::StartAfterTarget => {
            f.work_items[0].start_date = Some(day(5));
            f.work_items[0].target_date = Some(day(4));
        }
    }
    let got = block_on(MemoryDatabase::new().seed(&sys(), &f));
    match got {
        Err(DbError::Conflict(_)) if conflict => {}
        Err(DbError::Backend(_)) if !conflict => {}
        other => panic!("{wrong:?}: expected the refusal of a constraint, got {other:?}"),
    }
}
