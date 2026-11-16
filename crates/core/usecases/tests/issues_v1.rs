//! What the v1 API asks of a work item that the compat plane does not (issue #58), against the memory adapters: who may read and write
//! through it, a parent, a point and an integration key, a change that is whole, a project that is archived, a deletion with its own
//! rule and its cascade, and the workspace search.
//!
//! What must hold: any member of the project reads every work item, a guest too; an admin or a member writes, a guest does not;
//! a work item may have a parent (a work item of the project, itself too) where the compat plane refuses one (D-028); a change that
//! names a name keeps the parent, the point and the key; the v1 reads leave out a project that is archived and a change does not;
//! a work item is deleted by a project admin or by whoever made it, with the work items under it; and a search finds what a name, a
//! project identifier or a number says, in the caller's own projects.
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
use rstest::rstest;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

fn draft(name: &str) -> IssueDraft {
    IssueDraft { name: name.into(), ..IssueDraft::default() }
}

fn v1_writer(w: &World, who: Actor) -> Result<IssueAccess, IssueError> {
    block_on(v1_issue_writer(&w.app, &w.principal(who), &w.slug_a, w.project_a))
}

fn v1_reader(w: &World, who: Actor) -> Result<IssueAccess, IssueError> {
    block_on(v1_issue_reader(&w.app, &w.principal(who), &w.slug_a, w.project_a))
}

/// A work item made through the v1 API by `who`.
fn make(w: &World, who: Actor, d: IssueDraft) -> WorkItemRow {
    block_on(create_issue(&w.app, &v1_writer(w, who).unwrap(), d)).unwrap()
}

fn child_of(parent: &WorkItemRow, name: &str) -> IssueDraft {
    IssueDraft { parent_id: Some(parent.item.id.0), ..draft(name) }
}

/// What a read finds of a work item now, as an admin's v1 access sees it.
fn read(w: &World, id: WorkItemId) -> Result<WorkItemRow, IssueError> {
    block_on(get_issue(&w.app, &v1_reader(w, Actor::AdminA).unwrap(), id))
}

/// A change by `who` through the v1 API: the work item is found as a change finds it, then the asks are applied.
fn change(w: &World, who: Actor, id: WorkItemId, asks: Vec<Ask>) -> Result<(), IssueError> {
    let access = v1_writer(w, who)?;
    let before = block_on(issue_to_change(&w.app, &access, id))?;
    block_on(update_issue(&w.app, &access, before, IssueUpdate { asks, silent: false }))
}

fn delete(w: &World, who: Actor, id: WorkItemId) -> Result<(), IssueError> {
    block_on(delete_issue(&w.app, &v1_writer(w, who)?, id))
}

fn archive(w: &World) {
    let admin = w.principal(Actor::AdminA);
    let scope = WorkspaceScope::mint(w.ws_a, admin.user_id, Role::Admin);
    block_on(set_project_archived(&w.app, &scope, w.project_a, true)).unwrap();
}

fn page(w: &World, access: &IssueAccess) -> WorkItemPage {
    let order = WorkItemOrder { field: WorkItemOrderField::SequenceId, descending: false };
    block_on(list_issues(&w.app, access, IssueListQuery { order, limit: 100, page: 0, updated_after: None })).unwrap()
}

// --- who may -------------------------------------------------------------------------------------------------------------------

/// Any member of the project reads, an admin or a member writes, and a guest does not (the compat plane lets a guest change what they
/// made; the pin's v1 views do not). Anyone else is `Forbidden` to both.
#[rstest]
fn who_may_read_and_write_through_v1(
    #[values(Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::MemberB, Actor::Dual)] who: Actor,
) {
    let w = seeded();
    let writes = matches!(who, Actor::AdminA | Actor::MemberA | Actor::Dual);
    let reads = writes || who == Actor::GuestA;
    assert_eq!(v1_writer(&w, who).is_ok(), writes, "{who:?} may write");
    assert_eq!(v1_reader(&w, who).is_ok(), reads, "{who:?} may read");
    for refusal in [v1_writer(&w, who).err(), v1_reader(&w, who).err()].into_iter().flatten() {
        assert!(matches!(refusal, IssueError::Forbidden), "{who:?}: {refusal:?}");
    }
}

/// A guest of the compat plane sees only what they made (the project does not show guests everything); a guest of the v1 API sees
/// every work item.
#[test]
fn a_v1_guest_sees_every_work_item_where_a_compat_guest_does_not() {
    let w = seeded();
    let item = make(&w, Actor::AdminA, draft("made by the admin"));
    let compat = block_on(issue_reader(&w.app, &w.principal(Actor::GuestA), &w.slug_a, w.project_a)).unwrap();
    let v1 = v1_reader(&w, Actor::GuestA).unwrap();
    assert_eq!(page(&w, &compat).total, 0);
    assert!(matches!(block_on(get_issue(&w.app, &compat, item.item.id)), Err(IssueError::NotAllowedToView)));
    let seen = page(&w, &v1);
    assert_eq!((seen.total, seen.rows.len()), (1, 1));
    assert_eq!(block_on(get_issue(&w.app, &v1, item.item.id)).unwrap().item.id, item.item.id);
}

// --- a parent, a point and a key -----------------------------------------------------------------------------------------------

#[test]
fn a_v1_creation_takes_a_parent_a_point_and_a_key_and_the_compat_plane_still_refuses_a_parent() {
    let w = seeded();
    let parent = make(&w, Actor::AdminA, draft("parent"));
    let keyed = IssueDraft {
        point: Some(5),
        external_source: Some("gh".into()),
        external_id: Some("7".into()),
        ..child_of(&parent, "child")
    };
    let child = make(&w, Actor::MemberA, keyed);
    assert_eq!((child.attrs.parent_id, child.attrs.point), (Some(parent.item.id.0), Some(5)));
    assert_eq!((child.item.external_source.as_deref(), child.item.external_id.as_deref()), (Some("gh"), Some("7")));
    // what is stored is what a read finds, and the item is found by its number and by its key
    let access = v1_reader(&w, Actor::AdminA).unwrap();
    assert_eq!(read(&w, child.item.id).unwrap(), child);
    assert_eq!(block_on(issue_by_sequence(&w.app, &access, child.item.sequence_id)).unwrap().item.id, child.item.id);
    assert_eq!(block_on(issue_by_external(&w.app, &access, "gh", "7")).unwrap().item.id, child.item.id);
    for (source, id) in [("gh", "8"), ("jira", "7"), ("", "")] {
        assert!(matches!(block_on(issue_by_external(&w.app, &access, source, id)), Err(IssueError::NotFound)));
    }
    assert!(matches!(block_on(issue_by_sequence(&w.app, &access, 999)), Err(IssueError::NotFound)));
    // a parent that is no work item of the project is a problem to name
    let orphan = block_on(create_issue(
        &w.app,
        &v1_writer(&w, Actor::AdminA).unwrap(),
        IssueDraft { parent_id: Some(uuid(0xBAD)), ..draft("orphan") },
    ));
    assert!(matches!(&orphan, Err(IssueError::Fields(p)) if *p == vec![FieldProblem::Parent]), "{orphan:?}");
    // the compat plane has none to give (D-028), and says so for a parent that is there
    let compat = block_on(issue_writer(&w.app, &w.principal(Actor::AdminA), &w.slug_a, w.project_a)).unwrap();
    let refused = block_on(create_issue(&w.app, &compat, child_of(&parent, "compat child")));
    assert!(
        matches!(&refused, Err(IssueError::Fields(p)) if *p == vec![FieldProblem::ParentNotSupported]),
        "{refused:?}"
    );
}

/// A change is whole: what it does not name stays (the parent, the point and the key too); a parent can be cleared with null and can
/// be the work item itself, as the pin allows; and one that is not there is a problem to name.
#[test]
fn a_v1_change_sets_the_parent_the_point_and_the_key_and_a_name_alone_keeps_them() {
    let w = seeded();
    let (parent, other) = (make(&w, Actor::AdminA, draft("parent")), make(&w, Actor::AdminA, draft("other")));
    let item = make(&w, Actor::AdminA, draft("item"));
    let id = item.item.id;
    let ask = |asks: Vec<Ask>| change(&w, Actor::MemberA, id, asks).unwrap();
    ask(vec![
        Ask::Parent(Some(parent.item.id.0)),
        Ask::Point(Some(7)),
        Ask::ExternalSource(Some("gh".into())),
        Ask::ExternalId(Some("9".into())),
    ]);
    let now = read(&w, id).unwrap();
    assert_eq!((now.attrs.parent_id, now.attrs.point), (Some(parent.item.id.0), Some(7)));
    assert_eq!((now.item.external_source.as_deref(), now.item.external_id.as_deref()), (Some("gh"), Some("9")));
    // a change of the name alone keeps all of them
    ask(vec![Ask::Name { sent: "renamed".into(), name: "renamed".into() }]);
    let kept = read(&w, id).unwrap();
    assert_eq!(
        (kept.item.name.as_str(), kept.attrs.parent_id, kept.attrs.point, kept.item.external_id.as_deref()),
        ("renamed", Some(parent.item.id.0), Some(7), Some("9"))
    );
    // another parent, the item itself, and null
    ask(vec![Ask::Parent(Some(other.item.id.0))]);
    assert_eq!(read(&w, id).unwrap().attrs.parent_id, Some(other.item.id.0));
    ask(vec![Ask::Parent(Some(id.0))]);
    assert_eq!(read(&w, id).unwrap().attrs.parent_id, Some(id.0));
    ask(vec![Ask::Parent(None), Ask::Point(None), Ask::ExternalSource(None), Ask::ExternalId(None)]);
    let cleared = read(&w, id).unwrap();
    assert_eq!((cleared.attrs.parent_id, cleared.attrs.point, cleared.item.external_source), (None, None, None));
    // a parent that is not there is named, and nothing changed
    let got = change(&w, Actor::MemberA, id, vec![Ask::Parent(Some(uuid(0xBAD)))]);
    assert!(matches!(&got, Err(IssueError::Fields(p)) if *p == vec![FieldProblem::Parent]), "{got:?}");
    // and a guest does not change anything, whoever made it
    assert!(matches!(change(&w, Actor::GuestA, id, vec![]), Err(IssueError::Forbidden)));
}

// --- an archived project -------------------------------------------------------------------------------------------------------

/// The pin's `issue_objects` leaves out a project that is archived, so the v1 reads find nothing there; a change reads `Issue.objects`
/// and still finds the work item. The compat plane's list is unchanged.
#[test]
fn an_archived_project_has_no_work_items_for_the_v1_reads_but_a_change_still_finds_one() {
    let w = seeded();
    let item = make(&w, Actor::AdminA, draft("item"));
    archive(&w);
    let v1 = v1_reader(&w, Actor::AdminA).unwrap();
    let seen = page(&w, &v1);
    assert_eq!((seen.total, seen.rows.len(), seen.has_next), (0, 0, false));
    assert!(matches!(block_on(get_issue(&w.app, &v1, item.item.id)), Err(IssueError::NotFound)));
    assert!(matches!(block_on(issue_by_sequence(&w.app, &v1, item.item.sequence_id)), Err(IssueError::NotFound)));
    let compat = block_on(issue_reader(&w.app, &w.principal(Actor::AdminA), &w.slug_a, w.project_a)).unwrap();
    assert_eq!(page(&w, &compat).total, 1, "the compat plane lists it");
    let access = v1_writer(&w, Actor::AdminA).unwrap();
    assert_eq!(block_on(issue_to_change(&w.app, &access, item.item.id)).unwrap().item.id, item.item.id);
}

// --- deleting ------------------------------------------------------------------------------------------------------------------

/// A project admin deletes any work item and a member only one they made (`OnlyAdminOrCreator` otherwise, and nothing is deleted); a
/// work item that is not there is not found; each deletion is one event.
#[test]
fn a_work_item_is_deleted_by_a_project_admin_or_by_whoever_made_it() {
    let w = seeded();
    let (mine, theirs, admins) = (
        make(&w, Actor::MemberA, draft("the member's")),
        make(&w, Actor::AdminA, draft("the admin's")),
        make(&w, Actor::MemberA, draft("another of the member's")),
    );
    let before = block_on(outbox_depth(&w.app)).unwrap();
    // a member does not delete what the admin made
    assert!(matches!(delete(&w, Actor::MemberA, theirs.item.id), Err(IssueError::OnlyAdminOrCreator)));
    assert!(read(&w, theirs.item.id).is_ok());
    assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), before, "a refusal writes no event");
    // a guest is refused before that, by the access
    assert!(matches!(delete(&w, Actor::GuestA, mine.item.id), Err(IssueError::Forbidden)));
    // the maker deletes their own, and the admin deletes another member's
    delete(&w, Actor::MemberA, mine.item.id).unwrap();
    delete(&w, Actor::AdminA, admins.item.id).unwrap();
    assert!(matches!(read(&w, mine.item.id), Err(IssueError::NotFound)));
    assert!(matches!(read(&w, admins.item.id), Err(IssueError::NotFound)));
    assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), before + 2);
    // a second deletion and a work item that is nowhere are not found, and the admin's own is theirs to delete
    assert!(matches!(delete(&w, Actor::AdminA, mine.item.id), Err(IssueError::NotFound)));
    assert!(matches!(delete(&w, Actor::AdminA, WorkItemId(uuid(0xBAD))), Err(IssueError::NotFound)));
    delete(&w, Actor::AdminA, theirs.item.id).unwrap();
}

/// A deletion takes the work items under the work item, in turn; a work item that is its own parent, and two that are each other's,
/// do not keep the walk going; one item is the one event.
#[test]
fn a_deletion_takes_the_work_items_under_it_and_ends_on_a_loop() {
    let w = seeded();
    let root = make(&w, Actor::AdminA, draft("root"));
    let child = make(&w, Actor::AdminA, child_of(&root, "child"));
    let grandchild = make(&w, Actor::AdminA, child_of(&child, "grandchild"));
    let sibling = make(&w, Actor::AdminA, draft("sibling"));
    let (a, b) = (make(&w, Actor::AdminA, draft("a")), make(&w, Actor::AdminA, draft("b")));
    // a is its own parent; a and b are each other's
    change(&w, Actor::AdminA, a.item.id, vec![Ask::Parent(Some(a.item.id.0))]).unwrap();
    change(&w, Actor::AdminA, b.item.id, vec![Ask::Parent(Some(a.item.id.0))]).unwrap();
    change(&w, Actor::AdminA, a.item.id, vec![Ask::Parent(Some(b.item.id.0))]).unwrap();
    let before = block_on(outbox_depth(&w.app)).unwrap();

    delete(&w, Actor::AdminA, child.item.id).unwrap();
    assert!(matches!(read(&w, child.item.id), Err(IssueError::NotFound)));
    assert!(matches!(read(&w, grandchild.item.id), Err(IssueError::NotFound)));
    assert!(read(&w, root.item.id).is_ok() && read(&w, sibling.item.id).is_ok(), "what is not under it stays");
    delete(&w, Actor::AdminA, a.item.id).unwrap();
    assert!(matches!(read(&w, a.item.id), Err(IssueError::NotFound)));
    assert!(matches!(read(&w, b.item.id), Err(IssueError::NotFound)), "the loop goes whole");
    assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), before + 2, "one event for each deletion asked for");
    assert_eq!(page(&w, &v1_reader(&w, Actor::AdminA).unwrap()).total, 2, "the root and the sibling are left");
}

// --- the workspace search ------------------------------------------------------------------------------------------------------

fn search(
    w: &World,
    who: Actor,
    text: &str,
    numbers: &[i64],
    project: Option<ProjectId>,
    limit: u32,
) -> Vec<WorkItemHit> {
    let scope = block_on(authorize_workspace(&w.app, &w.principal(who), &w.slug_a)).unwrap();
    let q = WorkspaceSearch { text: text.into(), numbers: numbers.to_vec(), project, limit };
    block_on(search_issues(&w.app, &scope, q)).unwrap()
}

#[test]
fn a_search_finds_a_name_a_project_identifier_or_a_number_in_the_callers_own_projects() {
    let w = seeded();
    let (login, signup, other) = (
        make(&w, Actor::AdminA, draft("Fix the Login page")),
        make(&w, Actor::AdminA, draft("Sign-up form")),
        make(&w, Actor::AdminA, draft("100% done_ish")),
    );
    let names = |hits: &[WorkItemHit]| hits.iter().map(|h| h.name.clone()).collect::<Vec<_>>();
    // a name, in any case, and newest first
    let hits = search(&w, Actor::MemberA, "LOGIN", &[], None, 10);
    assert_eq!(names(&hits), ["Fix the Login page"]);
    assert_eq!(
        (hits[0].id, hits[0].sequence_id, hits[0].project_id),
        (login.item.id, login.item.sequence_id, w.project_a)
    );
    assert_eq!(hits[0].workspace_slug, w.slug_a);
    // a number, from the text, is a work item's number
    let by_number = search(&w, Actor::MemberA, "item 2", &[2], None, 10);
    assert_eq!(names(&by_number), ["Sign-up form"]);
    assert_eq!(by_number[0].id, signup.item.id);
    // the project's identifier is in what a hit says, and a search for it finds every work item of the project, newest first
    let identifier = hits[0].project_identifier.clone();
    let all = search(&w, Actor::MemberA, &identifier.to_lowercase(), &[], None, 10);
    assert_eq!(names(&all), ["100% done_ish", "Sign-up form", "Fix the Login page"]);
    // `%` and `_` are ordinary characters
    assert_eq!(names(&search(&w, Actor::MemberA, "100%", &[], None, 10)), ["100% done_ish"]);
    assert_eq!(names(&search(&w, Actor::MemberA, "done_ish", &[], None, 10)), ["100% done_ish"]);
    assert!(search(&w, Actor::MemberA, "%", &[], None, 10).iter().all(|h| h.id == other.item.id));
    assert!(search(&w, Actor::MemberA, "a_b", &[], None, 10).is_empty());
    // a limit cuts, a project that is not the work items' is empty, and nothing is found by an empty text
    assert_eq!(search(&w, Actor::MemberA, &identifier, &[], None, 2).len(), 2);
    assert!(search(&w, Actor::MemberA, "login", &[], Some(ProjectId(uuid(0xBAD))), 10).is_empty());
    assert_eq!(search(&w, Actor::MemberA, "login", &[], Some(w.project_a), 10).len(), 1);
    assert!(search(&w, Actor::MemberA, "", &[], None, 10).is_empty());
    // someone who is in the workspace and not in the project finds nothing, and neither does the project once it is archived
    assert!(search(&w, Actor::OutsiderA, "login", &[], None, 10).is_empty());
    archive(&w);
    assert!(search(&w, Actor::MemberA, "login", &[], None, 10).is_empty());
}
