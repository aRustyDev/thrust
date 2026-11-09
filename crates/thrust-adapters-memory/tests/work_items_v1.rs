//! What the v1 API (issue #58, migration 0015) needs of a work item in the memory adapter: a maker that may be missing, a parent, a
//! point and an integration key that are stored and checked, a lookup by number and by key, a deletion that takes the work items
//! under the work item, and the workspace search. The Postgres adapter is held to the same answers by `pg_adapter.rs` in
//! `thrust-adapters-selfhosted`.
//!
//! The rules, written out:
//!
//! * `insert_work_item_attrs` and `update_work_item` refuse (`Backend`, nothing written) a parent that is no work item, and a point
//!   outside 0 to 12; an update that gives an item a key that another live item of the project has is a `Conflict`.
//! * `work_item_by_sequence` and `work_item_by_external` find a live work item (not deleted, archived or a draft) of the scope's own
//!   project, and no other.
//! * `soft_delete_work_item` marks the work item and, in turn, the live work items under it (a loop and a self-parent end the walk);
//!   it finds an archived work item and a draft, which a read does not.
//! * `search_workspace_work_items` finds, among the live work items of the projects of the workspace that the caller is in and that
//!   are not archived, those whose name or project identifier contains the text in any case or whose number is one asked for,
//!   newest first, at most `limit`, with `%` and `_` as ordinary characters.
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
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;

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

fn scope(project: u128) -> Scope {
    let ws = if project == R { B } else { A };
    Scope::mint(wid(ws), pid(project), uid(1))
}

/// Workspace A (slug `a`) holds projects P (user 1 its admin, 2 a member, 3 a guest) and Q (user 2 a member); user 4 is in the
/// workspace and in neither. Workspace B holds R (user 1 a member).
fn world() -> MemoryDatabase {
    let db = MemoryDatabase::new();
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
        memberships: [(P, 1, Role::Admin), (P, 2, Role::Member), (P, 3, Role::Guest), (Q, 2, Role::Member)]
            .iter()
            .map(|(p, u, role)| SeedMembership {
                project_id: pid(*p),
                workspace_id: wid(A),
                user_id: uid(*u),
                role: *role,
            })
            .chain([SeedMembership { project_id: pid(R), workspace_id: wid(B), user_id: uid(1), role: Role::Member }])
            .collect(),
        ..SeedFixture::default()
    };
    block_on(db.seed(&SystemScope::mint(), &f)).unwrap();
    db
}

/// Insert a work item of `project`, numbered from its counter, with the attributes `attrs` makes of the bare ones, in a transaction of
/// its own, and commit it.
fn make(
    db: &MemoryDatabase,
    project: u128,
    edit: impl FnOnce(&mut WorkItem, &mut WorkItemAttrs),
) -> Result<WorkItemId, DbError> {
    let s = scope(project);
    let mut tx = block_on(db.begin(&s))?;
    let a = block_on(tx.allocate(&s))?;
    let mut item = WorkItem {
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
        // distinct across projects as well as within one, so that "newest first" has no ties to break
        created_at: at(a.sequence_id * 100 + project as i64),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    let mut attrs = WorkItemAttrs::bare(item.created_at);
    edit(&mut item, &mut attrs);
    let id = item.id;
    block_on(tx.insert_work_item(&s, &item))?;
    if let Err(e) = block_on(tx.insert_work_item_attrs(&s, id, &attrs)) {
        block_on(tx.rollback())?;
        return Err(e);
    }
    block_on(tx.commit())?;
    Ok(id)
}

fn item(db: &MemoryDatabase, project: u128, edit: impl FnOnce(&mut WorkItem, &mut WorkItemAttrs)) -> WorkItemId {
    make(db, project, edit).unwrap()
}

fn row(db: &MemoryDatabase, project: u128, id: WorkItemId) -> Option<WorkItemRow> {
    block_on(db.work_item_row(&scope(project), id)).unwrap()
}

fn delete(db: &MemoryDatabase, project: u128, id: WorkItemId) -> bool {
    let s = scope(project);
    let mut tx = block_on(db.begin(&s)).unwrap();
    let found = block_on(tx.soft_delete_work_item(&s, id, at(500))).unwrap();
    block_on(tx.commit()).unwrap();
    found
}

/// The whole update of an item as it stands, with the columns of the v1 API set as given.
fn whole(r: &WorkItemRow, parent: Option<Uuid>, point: Option<i32>, key: Option<(&str, &str)>) -> WorkItemUpdate {
    WorkItemUpdate {
        name: r.item.name.clone(),
        state_id: r.attrs.state_id,
        priority: r.attrs.priority,
        start_date: r.attrs.start_date,
        target_date: r.attrs.target_date,
        completed_at: r.attrs.completed_at,
        updated_at: at(100),
        updated_by: uid(1),
        description_html: r.attrs.description_html.clone(),
        assignee_ids: None,
        label_ids: None,
        external_source: key.map(|k| k.0.to_string()),
        external_id: key.map(|k| k.1.to_string()),
        parent_id: parent,
        point,
    }
}

fn update(db: &MemoryDatabase, project: u128, id: WorkItemId, u: &WorkItemUpdate) -> Result<bool, DbError> {
    let s = scope(project);
    let mut tx = block_on(db.begin(&s)).unwrap();
    let got = block_on(tx.update_work_item(&s, id, u));
    block_on(tx.commit()).unwrap();
    got
}

// --- what is stored -------------------------------------------------------------------------------------------------------------

#[test]
fn a_parent_a_point_and_a_key_are_stored_and_read_back_and_a_maker_may_be_missing() {
    let db = world();
    let parent = item(&db, P, |_, _| {});
    let child = item(&db, P, |w, a| {
        a.parent_id = Some(parent.0);
        a.point = Some(12);
        w.external_source = Some("gh".into());
        w.external_id = Some("7".into());
    });
    let r = row(&db, P, child).unwrap();
    assert_eq!((r.attrs.parent_id, r.attrs.point), (Some(parent.0), Some(12)));
    assert_eq!((r.item.external_source.as_deref(), r.item.external_id.as_deref()), (Some("gh"), Some("7")));
    // an item that nobody made is stored and read, and is not any one person's for a page asked for by maker
    let nobody = item(&db, P, |w, _| w.created_by = None);
    assert_eq!(row(&db, P, nobody).unwrap().item.created_by, None);
    let order = WorkItemOrder { field: WorkItemOrderField::SequenceId, descending: false };
    let by_one = WorkItemQuery { order, limit: 10, page: 0, updated_after: None, created_by: Some(uid(1)) };
    let page = block_on(db.work_item_page(&scope(P), &by_one)).unwrap();
    assert_eq!(page.rows.iter().map(|r| r.item.id).collect::<Vec<_>>(), vec![parent, child]);
}

#[rstest]
#[case::point_too_big(Some(13), None)]
#[case::point_negative(Some(-1), None)]
#[case::parent_is_no_item(None, Some(0xBAD))]
fn what_the_database_would_refuse_is_refused_and_writes_nothing(
    #[case] point: Option<i32>,
    #[case] parent: Option<u128>,
) {
    let db = world();
    let id = item(&db, P, |_, _| {});
    let parent = parent.map(Uuid::from_u128);
    // a creation: the item is not left behind either
    let got = make(&db, P, |_, a| {
        a.point = point;
        a.parent_id = parent;
    });
    assert!(matches!(got, Err(DbError::Backend(_))), "{got:?}");
    assert_eq!(block_on(db.list_work_items(&scope(P), Visibility::Visible)).unwrap().len(), 1);
    // an update
    let r = row(&db, P, id).unwrap();
    let got = update(&db, P, id, &whole(&r, parent, point, None));
    assert!(matches!(got, Err(DbError::Backend(_))), "{got:?}");
    assert_eq!(row(&db, P, id).unwrap(), r, "nothing was written");
}

#[test]
fn a_key_that_another_live_item_has_is_a_conflict_and_a_deleted_items_key_is_free() {
    let db = world();
    let holder = item(&db, P, |w, _| {
        w.external_source = Some("gh".into());
        w.external_id = Some("1".into());
    });
    let other = item(&db, P, |_, _| {});
    let r = row(&db, P, other).unwrap();
    let got = update(&db, P, other, &whole(&r, None, None, Some(("gh", "1"))));
    assert!(matches!(got, Err(DbError::Conflict(_))), "{got:?}");
    // the holder keeps its own, another source or id is another key, and a project of its own has its own keys
    let h = row(&db, P, holder).unwrap();
    assert!(update(&db, P, holder, &whole(&h, None, None, Some(("gh", "1")))).unwrap());
    assert!(update(&db, P, other, &whole(&r, None, None, Some(("jira", "1")))).unwrap());
    assert!(update(&db, P, other, &whole(&r, None, None, Some(("gh", "2")))).unwrap());
    let in_q = item(&db, Q, |_, _| {});
    assert!(update(&db, Q, in_q, &whole(&row(&db, Q, in_q).unwrap(), None, None, Some(("gh", "1")))).unwrap());
    // once the holder is deleted its key is free
    assert!(delete(&db, P, holder));
    assert!(update(&db, P, other, &whole(&r, None, None, Some(("gh", "1")))).unwrap());
}

// --- lookups --------------------------------------------------------------------------------------------------------------------

#[test]
fn an_item_is_found_by_its_number_and_by_its_key_when_it_is_live_and_of_the_scopes_own_project() {
    let db = world();
    let keyed = |s: &'static str, i: &'static str| {
        move |w: &mut WorkItem, _: &mut WorkItemAttrs| {
            w.external_source = Some(s.into());
            w.external_id = Some(i.into());
        }
    };
    let live = item(&db, P, keyed("gh", "1"));
    let (archived, draft, deleted) = (
        item(&db, P, |w, _| {
            w.archived_at = Some(at(1));
            w.external_source = Some("gh".into());
            w.external_id = Some("2".into());
        }),
        item(&db, P, |w, _| {
            w.is_draft = true;
            w.external_source = Some("gh".into());
            w.external_id = Some("3".into());
        }),
        item(&db, P, keyed("gh", "4")),
    );
    assert!(delete(&db, P, deleted));
    let number = |id: WorkItemId| {
        block_on(db.get_work_item(&scope(P), id, Visibility::WithDeleted)).unwrap().unwrap().sequence_id
    };
    let by_number =
        |project: u128, n: i64| block_on(db.work_item_by_sequence(&scope(project), n)).unwrap().map(|r| r.item.id);
    let by_key = |project: u128, s: &str, i: &str| {
        block_on(db.work_item_by_external(&scope(project), s, i)).unwrap().map(|r| r.item.id)
    };
    assert_eq!(by_number(P, number(live)), Some(live));
    assert_eq!(by_key(P, "gh", "1"), Some(live));
    for gone in [archived, draft, deleted] {
        assert_eq!(by_number(P, number(gone)), None, "{gone:?} is not live");
    }
    for (i, gone) in [(2, archived), (3, draft), (4, deleted)] {
        assert_eq!(by_key(P, "gh", &i.to_string()), None, "{gone:?} is not live");
    }
    // the key is exact, the number is the project's own, and another project's scope finds neither
    assert_eq!((by_key(P, "GH", "1"), by_key(P, "gh", "11"), by_key(P, "", "")), (None, None, None));
    assert_eq!(
        (by_number(P, 99), by_number(Q, number(live)), by_key(Q, "gh", "1"), by_key(R, "gh", "1")),
        (None, None, None, None)
    );
}

// --- deleting -------------------------------------------------------------------------------------------------------------------

#[test]
fn a_deleted_item_takes_the_items_under_it_and_finds_an_archived_one_and_a_draft() {
    let db = world();
    let under = |p: WorkItemId| move |_: &mut WorkItem, a: &mut WorkItemAttrs| a.parent_id = Some(p.0);
    let root = item(&db, P, |_, _| {});
    let child = item(&db, P, under(root));
    let grandchild = item(&db, P, under(child));
    let sibling = item(&db, P, |_, _| {});
    let elsewhere = item(&db, Q, |_, _| {});
    assert!(delete(&db, P, child));
    assert!(row(&db, P, child).is_none() && row(&db, P, grandchild).is_none());
    assert!(row(&db, P, root).is_some() && row(&db, P, sibling).is_some() && row(&db, Q, elsewhere).is_some());
    // a second deletion, an item that is nowhere and another project's item are not there
    assert!(!delete(&db, P, child));
    assert!(!delete(&db, P, WorkItemId(Uuid::from_u128(0xBAD))));
    assert!(!delete(&db, P, elsewhere));
    // an archived item and a draft are there to delete, though a read does not find them
    let archived = item(&db, P, |w, _| w.archived_at = Some(at(1)));
    let draft = item(&db, P, |w, _| w.is_draft = true);
    assert!(delete(&db, P, archived) && delete(&db, P, draft));
    assert!(!delete(&db, P, archived));
}

/// An item that is deleted twice in one transaction (the second call does not see the first: it reads what is committed) keeps the
/// time of the first; and the items under it are marked with the time their parent was, whichever of them is asked for first.
#[test]
fn a_deleted_item_keeps_the_time_it_was_first_deleted() {
    let db = world();
    let (id, other) = (item(&db, P, |_, _| {}), item(&db, P, |_, _| {}));
    let child = item(&db, P, |_, a| a.parent_id = Some(id.0));
    let s = scope(P);
    let mut tx = block_on(db.begin(&s)).unwrap();
    assert!(block_on(tx.soft_delete_work_item(&s, id, at(500))).unwrap());
    assert!(block_on(tx.soft_delete_work_item(&s, id, at(600))).unwrap());
    assert!(block_on(tx.soft_delete_work_item(&s, other, at(700))).unwrap());
    block_on(tx.commit()).unwrap();
    let deleted_at =
        |item: WorkItemId| block_on(db.get_work_item(&s, item, Visibility::WithDeleted)).unwrap().unwrap().deleted_at;
    assert_eq!(deleted_at(id), Some(at(500)), "the first time is kept");
    assert_eq!(deleted_at(child), Some(at(500)), "the child goes with the first walk");
    assert_eq!(deleted_at(other), Some(at(700)), "and another item has its own time");
}

proptest! {
    #![proptest_config(proptest_config(96))]

    /// In any forest of work items (a parent may be any item, itself and a loop included), deleting one deletes exactly the items that
    /// reach it by parents. The model is the closure, computed here.
    #[test]
    fn prop_deleting_an_item_deletes_exactly_what_reaches_it_by_parents(
        (parents, victim) in (1usize..9).prop_flat_map(|n| (prop::collection::vec(prop::option::of(0..n), n), 0..n)),
    ) {
        let db = world();
        // the items first, then each one's parent (a later item may be the parent of an earlier one)
        let ids: Vec<WorkItemId> = (0..parents.len()).map(|_| item(&db, P, |_, _| {})).collect();
        for (id, parent) in ids.iter().zip(&parents) {
            let r = row(&db, P, *id).unwrap();
            let changed = update(&db, P, *id, &whole(&r, parent.map(|p| ids[p].0), None, None)).unwrap();
            prop_assert!(changed);
        }
        let mut gone = std::collections::BTreeSet::from([victim]);
        loop {
            let before = gone.len();
            for (i, parent) in parents.iter().enumerate() {
                if parent.is_some_and(|p| gone.contains(&p)) {
                    gone.insert(i);
                }
            }
            if gone.len() == before {
                break;
            }
        }
        prop_assert!(delete(&db, P, ids[victim]));
        for (i, id) in ids.iter().enumerate() {
            prop_assert_eq!(row(&db, P, *id).is_none(), gone.contains(&i), "item {}", i);
        }
    }
}

// --- the workspace search -------------------------------------------------------------------------------------------------------

fn search(
    db: &MemoryDatabase,
    user: u128,
    text: &str,
    numbers: &[i64],
    project: Option<u128>,
    limit: u32,
) -> Vec<String> {
    let scope = WorkspaceScope::mint(wid(A), uid(user), Role::Member);
    let q = WorkspaceSearch { text: text.into(), numbers: numbers.to_vec(), project: project.map(pid), limit };
    block_on(db.search_workspace_work_items(&scope, &q)).unwrap().into_iter().map(|h| h.name).collect()
}

fn named(name: &'static str) -> impl FnOnce(&mut WorkItem, &mut WorkItemAttrs) {
    move |w, _| w.name = name.into()
}

#[test]
fn a_search_finds_what_a_name_an_identifier_or_a_number_says_in_the_callers_own_live_projects() {
    let db = world();
    item(&db, P, named("Alpha login"));
    item(&db, P, named("Beta sign-up"));
    item(&db, P, named("100% done_ish"));
    item(&db, P, |w, _| {
        w.name = "Archived login".into();
        w.archived_at = Some(at(1));
    });
    item(&db, P, |w, _| {
        w.name = "Draft login".into();
        w.is_draft = true;
    });
    let gone = item(&db, P, named("Deleted login"));
    assert!(delete(&db, P, gone));
    item(&db, Q, named("Q login"));
    item(&db, R, named("R login"));

    // a name, in any case, newest first; user 2 is in P and Q, user 1 in P alone, user 3 a guest of P
    assert_eq!(search(&db, 2, "LOGIN", &[], None, 10), ["Q login", "Alpha login"]);
    assert_eq!(search(&db, 1, "login", &[], None, 10), ["Alpha login"]);
    assert_eq!(search(&db, 3, "login", &[], None, 10), ["Alpha login"]);
    // someone in the workspace and in no project, and someone in another workspace, find nothing
    assert!(search(&db, 4, "login", &[], None, 10).is_empty());
    assert!(search(&db, 5, "login", &[], None, 10).is_empty());
    // a number is a work item's number in any project the caller is in, and a project narrows it
    assert_eq!(search(&db, 2, "no such text", &[1], None, 10), ["Q login", "Alpha login"]);
    assert_eq!(search(&db, 2, "no such text", &[1], Some(Q), 10), ["Q login"]);
    assert!(search(&db, 1, "login", &[], Some(Q), 10).is_empty(), "a project the caller is not in");
    // `%` and `_` are ordinary characters
    assert_eq!(search(&db, 1, "100%", &[], None, 10), ["100% done_ish"]);
    assert_eq!(search(&db, 1, "done_ish", &[], None, 10), ["100% done_ish"]);
    assert!(search(&db, 1, "a_b", &[], None, 10).is_empty());
    // a limit cuts
    assert_eq!(search(&db, 2, "login", &[], None, 1), ["Q login"]);
    // the identifier of a project is in what a hit says and what a search finds
    let ws = WorkspaceScope::mint(wid(A), uid(1), Role::Admin);
    let identifier = block_on(db.project_detail(&ws, pid(P))).unwrap().unwrap().listing.project.identifier;
    let hit = block_on(db.search_workspace_work_items(
        &WorkspaceScope::mint(wid(A), uid(1), Role::Member),
        &WorkspaceSearch { text: identifier.to_lowercase(), numbers: vec![], project: None, limit: 10 },
    ))
    .unwrap();
    assert_eq!(hit.len(), 3, "every live item of P, whatever it is called");
    assert!(hit
        .iter()
        .all(|h| h.project_identifier == identifier && h.project_id == pid(P) && h.workspace_slug.0 == "a"));
    // an archived project is not searched
    let mut p = block_on(db.project_detail(&ws, pid(Q))).unwrap().unwrap().listing.project;
    p.archived_at = Some(at(9));
    assert!(block_on(db.update_project(&ws, &p)).unwrap());
    assert_eq!(search(&db, 2, "login", &[], None, 10), ["Alpha login"]);
}
