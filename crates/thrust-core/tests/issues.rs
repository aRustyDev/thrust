//! Work items as the compat plane serves them (issue #44, the work item stage): who may make, list and open them, what a creation
//! checks and gives, and what a guest sees, against the memory adapters.
//!
//! What must hold: an admin or a member of the project makes a work item (a guest and everyone outside the project are
//! `Forbidden`) and any member reads them; a creation names a state, people and labels that are there, the dates in order, no parent;
//! it is numbered from the project's counter, put where the pin puts it, given the default state, assigned as the pin assigns, and
//! its `work_item.created` event is written with it; and a guest of a project that does not show them everything reads only what
//! they made.

mod common;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::chrono::NaiveDate;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const STATE_BACKLOG: u128 = 0x5001;
const STATE_TODO: u128 = 0x5002;
const STATE_DONE: u128 = 0x5003;
const STATE_OF_B: u128 = 0x5101;

fn uuid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// The world with states and labels for both projects: project A's `Backlog` (the default), `Todo` and `Done` (of the completed
/// group), two labels; project B's state and label.
fn seeded() -> World {
    let w = world();
    let state = |project: ProjectId, ws: WorkspaceId, id: u128, name: &str, group: StateGroup, sequence: f64, is_default: bool| SeedState {
        id: uuid(id),
        project_id: project,
        workspace_id: ws,
        name: name.into(),
        color: "#000000".into(),
        sequence,
        group,
        is_default,
    };
    let label = |project: ProjectId, ws: WorkspaceId, n: u128| SeedLabel {
        id: uuid(0x7000 + n),
        project_id: project,
        workspace_id: ws,
        name: format!("l{n}"),
        color: "#fff".into(),
        parent_id: None,
        sort_order: n as f64,
    };
    let f = SeedFixture {
        states: vec![
            state(w.project_a, w.ws_a, STATE_BACKLOG, "Backlog", StateGroup::Backlog, 10_000.0, true),
            state(w.project_a, w.ws_a, STATE_TODO, "Todo", StateGroup::Unstarted, 20_000.0, false),
            state(w.project_a, w.ws_a, STATE_DONE, "Done", StateGroup::Completed, 30_000.0, false),
            state(w.project_b, w.ws_b, STATE_OF_B, "Backlog", StateGroup::Backlog, 10_000.0, true),
        ],
        labels: vec![label(w.project_a, w.ws_a, 1), label(w.project_a, w.ws_a, 2), label(w.project_b, w.ws_b, 3)],
        ..SeedFixture::default()
    };
    block_on(w.app.ports.db.seed(&SystemScope::mint(), &f)).unwrap();
    w
}

fn draft(name: &str) -> IssueDraft {
    IssueDraft { name: name.into(), ..IssueDraft::default() }
}

fn writer(w: &World, who: Actor) -> Result<IssueAccess, IssueError> {
    block_on(issue_writer(&w.app, &w.principal(who), &w.slug_a, w.project_a))
}

fn reader(w: &World, who: Actor) -> Result<IssueAccess, IssueError> {
    block_on(issue_reader(&w.app, &w.principal(who), &w.slug_a, w.project_a))
}

fn made(w: &World, who: Actor, d: IssueDraft) -> Result<WorkItemRow, IssueError> {
    let access = writer(w, who)?;
    block_on(create_issue(&w.app, &access, d))
}

fn page(w: &World, who: Actor) -> Result<WorkItemPage, IssueError> {
    let access = reader(w, who)?;
    let order = WorkItemOrder { field: WorkItemOrderField::SequenceId, descending: false };
    block_on(list_issues(&w.app, &access, IssueListQuery { order, limit: 100, page: 0, updated_after: None }))
}

fn user(w: &World, who: Actor) -> UserId {
    w.principal(who).user_id
}

// --- who may -------------------------------------------------------------------------------------------------------------------

/// Spec (the pin's `IssueViewSet`): an admin or a member of the project makes a work item and a guest does not; any member, a guest
/// too, reads them; someone who is no member of the project (in its workspace or not) does neither, and is told the same.
#[rstest]
fn who_may_make_and_who_may_read(#[values(Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::MemberB, Actor::Dual)] who: Actor) {
    let w = seeded();
    let writes = matches!(who, Actor::AdminA | Actor::MemberA | Actor::Dual);
    let reads = writes || who == Actor::GuestA;
    assert_eq!(writer(&w, who).is_ok(), writes, "{who:?} may make");
    assert_eq!(reader(&w, who).is_ok(), reads, "{who:?} may read");
    for refusal in [writer(&w, who).err(), reader(&w, who).err()].into_iter().flatten() {
        assert!(matches!(refusal, IssueError::Forbidden), "{who:?}: {refusal:?}");
    }
    // and a project that is not in the slug's workspace is no different
    let other = block_on(issue_reader(&w.app, &w.principal(who), &w.slug_b, w.project_a));
    assert!(matches!(other, Err(IssueError::Forbidden)), "{who:?}: {other:?}");
}

// --- making one ----------------------------------------------------------------------------------------------------------------

/// Spec: a work item is numbered from 1 and put at 65535 and 10000 further for each one after (the pin's places), by whoever makes
/// it, and what comes back is what a read finds. Its `work_item.created` event is in the outbox with it.
proptest! {
    #![proptest_config(proptest_config(48))]

    #[test]
    fn prop_work_items_are_numbered_and_placed_one_after_another(makers in prop::collection::vec(prop_oneof![Just(Actor::AdminA), Just(Actor::MemberA), Just(Actor::Dual)], 1..8)) {
        let w = seeded();
        let mut rows = Vec::new();
        for (i, who) in makers.iter().enumerate() {
            let row = made(&w, *who, draft(&format!("item {i}"))).unwrap();
            prop_assert_eq!((row.item.sequence_id, row.item.sort_order), (i as i64 + 1, 65_535 + 10_000 * i as i64));
            prop_assert_eq!(row.item.created_by, user(&w, *who));
            rows.push(row);
        }
        let got = page(&w, Actor::AdminA).unwrap();
        prop_assert_eq!(got.rows, rows);
        prop_assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), makers.len() as u64);
    }
}

/// Spec: the work item takes the project's default state when it names none, and has `completed_at` when its state is of the
/// completed group (and only then); the priority is `none`, the description the empty paragraph, nobody updated it.
#[rstest]
fn a_new_work_item_has_a_state_a_priority_and_a_description_the_pins_defaults(#[values(None, Some(STATE_BACKLOG), Some(STATE_TODO), Some(STATE_DONE))] state: Option<u128>) {
    let w = seeded();
    let row = made(&w, Actor::MemberA, IssueDraft { state_id: state.map(uuid), ..draft("n") }).unwrap();
    let want = state.unwrap_or(STATE_BACKLOG);
    assert_eq!(row.attrs.state_id, Some(uuid(want)));
    assert_eq!(row.state_group, Some(if want == STATE_DONE { StateGroup::Completed } else if want == STATE_TODO { StateGroup::Unstarted } else { StateGroup::Backlog }));
    assert_eq!(row.attrs.completed_at.is_some(), want == STATE_DONE, "completed_at follows the completed group");
    assert_eq!((row.attrs.priority, row.attrs.description_html.as_str(), row.attrs.updated_by), (Priority::None, "<p></p>", None));
    assert_eq!(row.attrs.updated_at, row.item.created_at);
    assert_eq!(row.item.name, "n");
}

/// Spec: a name is trimmed; the description, the priority and the dates are kept as given.
#[test]
fn what_a_creation_names_is_kept() {
    let w = seeded();
    let d = IssueDraft {
        description_html: Some("<p>hi</p>".into()),
        priority: Some(Priority::Urgent),
        start_date: NaiveDate::from_ymd_opt(2026, 11, 1),
        target_date: NaiveDate::from_ymd_opt(2026, 11, 1),
        ..draft("  padded  ")
    };
    let row = made(&w, Actor::MemberA, d).unwrap();
    assert_eq!(row.item.name, "padded");
    assert_eq!((row.attrs.description_html.as_str(), row.attrs.priority), ("<p>hi</p>", Priority::Urgent));
    assert_eq!((row.attrs.start_date, row.attrs.target_date), (NaiveDate::from_ymd_opt(2026, 11, 1), NaiveDate::from_ymd_opt(2026, 11, 1)));
}

/// Spec (`validate`): a start date after the target date is refused, the same day is not, and a refused creation writes nothing.
#[rstest]
fn the_dates_must_be_in_order(#[values(0, 1, 2)] start: u32, #[values(0, 1, 2)] target: u32) {
    let w = seeded();
    let day = |n: u32| NaiveDate::from_ymd_opt(2026, 11, 1 + n);
    let got = made(&w, Actor::MemberA, IssueDraft { start_date: day(start), target_date: day(target), ..draft("d") });
    assert_eq!(matches!(got, Err(IssueError::StartAfterTarget)), start > target, "{start} {target}: {got:?}");
    assert_eq!(got.is_ok(), start <= target);
    assert_eq!(page(&w, Actor::AdminA).unwrap().total, u64::from(start <= target));
}

/// Spec: a state that is none of the project's (another project's, or none) is a problem with the state, the parent that is none
/// of its work items one with the parent, an assignee who is no user one with their place in the list, a label that is none of
/// the project's one with its place; all are found together, in the order the request names them, and nothing is written.
#[test]
fn what_a_creation_names_must_be_there_and_every_problem_is_found() {
    let w = seeded();
    let nobody = UserId(uuid(0xDEAD));
    let d = IssueDraft {
        state_id: Some(uuid(STATE_OF_B)),
        parent_id: Some(uuid(0xBAD)),
        assignee_ids: vec![user(&w, Actor::MemberA), nobody, nobody],
        label_ids: vec![uuid(0x7003), uuid(0x7001), uuid(0xDEAD)],
        ..draft("n")
    };
    let Err(IssueError::Fields(problems)) = made(&w, Actor::MemberA, d) else { panic!("not refused as fields") };
    assert_eq!(
        problems,
        vec![FieldProblem::State, FieldProblem::Parent, FieldProblem::Assignee(1), FieldProblem::Assignee(2), FieldProblem::Label(0), FieldProblem::Label(2)]
    );
    assert_eq!(page(&w, Actor::AdminA).unwrap().total, 0, "a refused creation makes nothing");
    assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), 0);
}

/// Spec (deviation D-028): a parent that is one of the project's work items is refused in the words of the work item, not as
/// missing: work items have no parent here yet.
#[test]
fn a_parent_that_exists_is_refused_as_not_supported() {
    let w = seeded();
    let parent = made(&w, Actor::MemberA, draft("parent")).unwrap();
    let got = made(&w, Actor::MemberA, IssueDraft { parent_id: Some(parent.item.id.0), ..draft("child") });
    assert!(matches!(&got, Err(IssueError::Fields(p)) if *p == vec![FieldProblem::ParentNotSupported]), "{got:?}");
}

/// Spec (`validate` and `create`): only those named who are members of the project, not guests, are assigned (someone in the workspace
/// and not the project, a guest and a duplicate are left out without a word); when none is left the project's default assignee is,
/// if they are one such; the labels are the project's, once each.
#[test]
fn assignees_are_the_named_who_may_be_and_else_the_default_assignee() {
    let w = seeded();
    let (member, admin, guest, outsider) = (user(&w, Actor::MemberA), user(&w, Actor::AdminA), user(&w, Actor::GuestA), user(&w, Actor::OutsiderA));
    let row = made(&w, Actor::MemberA, IssueDraft { assignee_ids: vec![admin, guest, outsider, member, admin], ..draft("a") }).unwrap();
    let mut want = vec![admin, member];
    want.sort();
    assert_eq!(row.attrs.assignee_ids, want, "the guest and the outsider are left out, each once, in id order");

    // nobody left: the project's default assignee, when they may be assigned
    let admin_scope = WorkspaceScope::mint(w.ws_a, admin, Role::Admin);
    let set_default = |who: Option<UserId>| {
        block_on(update_project(&w.app, &admin_scope, w.project_a, ProjectPatch { default_assignee: Some(who), ..ProjectPatch::default() })).unwrap();
    };
    set_default(Some(member));
    let none_left = made(&w, Actor::MemberA, IssueDraft { assignee_ids: vec![guest, outsider], ..draft("b") }).unwrap();
    assert_eq!(none_left.attrs.assignee_ids, vec![member]);
    let none_named = made(&w, Actor::MemberA, draft("c")).unwrap();
    assert_eq!(none_named.attrs.assignee_ids, vec![member]);
    let named = made(&w, Actor::MemberA, IssueDraft { assignee_ids: vec![admin], ..draft("d") }).unwrap();
    assert_eq!(named.attrs.assignee_ids, vec![admin], "a named assignee is not joined by the default");
    // a default assignee who is a guest of the project is not assigned
    set_default(Some(guest));
    assert_eq!(made(&w, Actor::MemberA, draft("e")).unwrap().attrs.assignee_ids, vec![]);
    set_default(None);
    assert_eq!(made(&w, Actor::MemberA, draft("f")).unwrap().attrs.assignee_ids, vec![]);

    let labelled = made(&w, Actor::MemberA, IssueDraft { label_ids: vec![uuid(0x7002), uuid(0x7001), uuid(0x7002)], ..draft("g") }).unwrap();
    assert_eq!(labelled.attrs.label_ids, vec![uuid(0x7001), uuid(0x7002)]);
}

// --- reading -------------------------------------------------------------------------------------------------------------------

/// Spec (`list` and `retrieve`): a guest of a project that does not show guests everything reads the work items they made and no
/// others, and one they did not make is `NotAllowedToView` (a work item that is not there is `NotFound` to them as to anyone);
/// once the project shows guests everything, they read all; a member reads all either way.
#[rstest]
fn a_guest_reads_only_what_they_made_unless_the_project_shows_them_everything(#[values(false, true)] guests_see_all: bool) {
    let w = seeded();
    let admins = made(&w, Actor::AdminA, draft("by the admin")).unwrap();
    let members = made(&w, Actor::MemberA, draft("by a member")).unwrap();
    if guests_see_all {
        let admin_scope = WorkspaceScope::mint(w.ws_a, user(&w, Actor::AdminA), Role::Admin);
        block_on(update_project(&w.app, &admin_scope, w.project_a, ProjectPatch { guest_view_all_features: Some(true), ..ProjectPatch::default() })).unwrap();
    }
    let ids = |who: Actor| page(&w, who).unwrap().rows.iter().map(|r| r.item.id).collect::<Vec<_>>();
    assert_eq!(ids(Actor::MemberA), vec![admins.item.id, members.item.id], "a member reads all");
    assert_eq!(ids(Actor::GuestA), if guests_see_all { vec![admins.item.id, members.item.id] } else { vec![] });
    let opened = block_on(get_issue(&w.app, &reader(&w, Actor::GuestA).unwrap(), admins.item.id));
    assert_eq!(opened.is_ok(), guests_see_all, "{opened:?}");
    if !guests_see_all {
        assert!(matches!(opened, Err(IssueError::NotAllowedToView)));
    }
    let gone = block_on(get_issue(&w.app, &reader(&w, Actor::GuestA).unwrap(), WorkItemId(uuid(0xBAD))));
    assert!(matches!(gone, Err(IssueError::NotFound)), "{gone:?}");
    // and the total a guest is told is the count of what they may see
    assert_eq!(page(&w, Actor::GuestA).unwrap().total, if guests_see_all { 2 } else { 0 });
}

/// Spec: a guest who made a work item reads it though the project does not show them the rest (a member who is made a guest keeps
/// their own work). Made by a member of the project here by seeding a work item the guest created, which no route makes.
#[test]
fn a_guest_reads_the_work_item_they_made() {
    let w = seeded();
    let guest = user(&w, Actor::GuestA);
    // the guest's own item: inserted as the guest, through the port, as the slice would have if it let a guest write
    let scope = block_on(authorize(&w.app, &w.principal(Actor::GuestA), &w.slug_a, w.project_a, Permission::ReadWorkItems)).unwrap();
    let mut tx = block_on(w.app.ports.db.begin(&scope)).unwrap();
    let alloc = block_on(tx.allocate(&scope)).unwrap();
    let item = WorkItem {
        id: WorkItemId(uuid(0x9100)),
        workspace_id: w.ws_a,
        project_id: w.project_a,
        sequence_id: alloc.sequence_id,
        sort_order: alloc.sort_order,
        name: "the guest's".into(),
        description: None,
        external_source: None,
        external_id: None,
        created_by: guest,
        created_at: w.app.ports.clock.now(),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    block_on(tx.insert_work_item(&scope, &item)).unwrap();
    block_on(tx.commit()).unwrap();
    made(&w, Actor::MemberA, draft("someone else's")).unwrap();
    let seen = page(&w, Actor::GuestA).unwrap();
    assert_eq!(seen.rows.iter().map(|r| r.item.id).collect::<Vec<_>>(), vec![item.id]);
    assert_eq!(seen.total, 1);
    assert!(block_on(get_issue(&w.app, &reader(&w, Actor::GuestA).unwrap(), item.id)).is_ok());
}
