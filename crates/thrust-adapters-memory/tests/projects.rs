//! Projects in the memory adapter (issue #44, migration 0012): making one with its members and states, renaming it, who is in it,
//! its states and labels, and the workspace's counts. The Postgres adapter is held to the same answers by the generated
//! sequences in `thrust-adapters-selfhosted/tests/model`; what that model cannot reach (several projects at once) is here.
//!
//! The rules, written out:
//!
//! * `insert_project` makes the project, its members (each with a settings row) and its states together or not at all: a project
//!   id, a key or a name that is taken, a user who does not exist, a member who is no member of the workspace (D-026), a state name
//!   twice or two default states refuse all of it. A
//!   member's place in their own list is 10000 below the lowest they hold in the workspace, or 65535 when they hold none.
//! * `project_clashes` is exact (`Plane` and `plane` are two names) and leaves out the project being changed.
//! * `update_project` replaces everything but the id, the workspace and the creation; a name or key another project has is a
//!   conflict, one's own is not; a project that is not in the scope's workspace is not there.
//! * `project_detail` says the caller's role only when their row names the project's own workspace, who is in it newest first, and
//!   the number the next work item will have.
//! * `project_members` leaves out people who are no longer active members of the workspace.
//! * the states lists leave out the intake (triage) state; `workspace_states` holds only the projects the caller is in that are not
//!   archived; `project_stats` counts every project of the workspace, whoever asks.

mod common;

use common::proptest_config;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::json;
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;

fn at(n: i64) -> Timestamp {
    DateTime::from_timestamp(1_760_000_000 + n, 0).unwrap()
}

fn sys() -> SystemScope {
    SystemScope::mint()
}

fn uid(n: u128) -> UserId {
    UserId(Uuid::from_u128(n))
}

fn wid(n: u128) -> WorkspaceId {
    WorkspaceId(Uuid::from_u128(n))
}

fn pid(n: u128) -> ProjectId {
    ProjectId(Uuid::from_u128(n))
}

const A: u128 = 0xA;
const B: u128 = 0xB;

fn wscope(ws: u128, user: u128, role: Role) -> WorkspaceScope {
    WorkspaceScope::mint(wid(ws), uid(user), role)
}

fn pscope(ws: u128, project: u128, user: u128) -> Scope {
    Scope::mint(wid(ws), pid(project), uid(user))
}

/// Users 1 to 8 exist, and workspaces A and B exist with all of them active members.
fn world() -> MemoryDatabase {
    let db = MemoryDatabase::new();
    let owner = uid(1);
    for (n, slug) in [(A, "a"), (B, "b")] {
        let id = wid(n);
        let workspace = Workspace {
            id,
            slug: WorkspaceSlug::new(slug),
            name: slug.into(),
            owner_id: None,
            organization_size: None,
            timezone: "UTC".into(),
            background_color: "#000000".into(),
            created_at: at(0),
            updated_at: at(0),
        };
        let member = WorkspaceMember {
            id: Uuid::from_u128(0x1000 + n),
            workspace_id: id,
            user_id: owner,
            role: Role::Admin,
            company_role: None,
            is_active: true,
            props: json!({}),
            created_at: at(0),
            updated_at: at(0),
        };
        // the owner is a user from the seed below, so the workspace goes in after it
        let f = SeedFixture { users: (1..=8).map(uid).collect(), ..SeedFixture::default() };
        block_on(db.seed(&sys(), &f)).unwrap();
        block_on(db.insert_workspace(&NewWorkspace { workspace, owner: member })).unwrap();
    }
    let f = SeedFixture {
        workspace_members: [A, B].iter().flat_map(|w| (2..=8).map(move |u| SeedWorkspaceMember { workspace_id: wid(*w), user_id: uid(u) })).collect(),
        ..SeedFixture::default()
    };
    block_on(db.seed(&sys(), &f)).unwrap();
    db
}

/// A project `n` of workspace `ws` made by `creator` at second `created`, with `others` as its other admins, named `p<n>` with the key
/// `P<n>` and with the pin's six states.
fn new_project(ws: u128, n: u128, creator: u128, others: &[u128], created: i64) -> NewProject {
    let project = {
        let mut p = ProjectRow::bare(pid(n), wid(ws), format!("p{n}"), at(created));
        p.identifier = format!("P{n}");
        p.created_by = Some(uid(creator));
        p
    };
    let members = std::iter::once(creator)
        .chain(others.iter().copied())
        .enumerate()
        .map(|(i, u)| {
            let t = at(created) + thrust_contracts::chrono::TimeDelta::microseconds(i as i64);
            NewProjectMember {
                member: ProjectMemberRow {
                    id: Uuid::from_u128((n << 16) | u),
                    project_id: pid(n),
                    workspace_id: wid(ws),
                    user_id: uid(u),
                    role: Role::Admin,
                    comment: None,
                    view_props: json!({}),
                    default_props: json!({}),
                    preferences: json!({}),
                    sort_order: 65535.0,
                    created_by: Some(uid(creator)),
                    created_at: t,
                    updated_at: t,
                },
                properties: ProjectUserProperties {
                    id: Uuid::from_u128((n << 32) | u),
                    project_id: pid(n),
                    workspace_id: wid(ws),
                    user_id: uid(u),
                    filters: json!({}),
                    display_filters: json!({}),
                    display_properties: json!({}),
                    rich_filters: json!({}),
                    preferences: json!({}),
                    // the adapter replaces this
                    sort_order: -1.0,
                    created_at: t,
                    updated_at: t,
                },
            }
        })
        .collect();
    let states = [
        ("Backlog", 15000.0, StateGroup::Backlog, true),
        ("Todo", 25000.0, StateGroup::Unstarted, false),
        ("In Progress", 35000.0, StateGroup::Started, false),
        ("Done", 45000.0, StateGroup::Completed, false),
        ("Cancelled", 55000.0, StateGroup::Cancelled, false),
        ("Triage", 65000.0, StateGroup::Triage, false),
    ]
    .iter()
    .enumerate()
    .map(|(i, (name, sequence, group, is_default))| StateRow {
        id: Uuid::from_u128((n << 48) | i as u128),
        workspace_id: wid(ws),
        project_id: pid(n),
        name: (*name).into(),
        description: String::new(),
        color: "#000000".into(),
        sequence: *sequence,
        group: *group,
        is_default: *is_default,
        created_by: Some(uid(creator)),
        created_at: at(created),
        updated_at: at(created),
    })
    .collect();
    NewProject { project, members, states }
}

fn make(db: &MemoryDatabase, p: &NewProject) {
    block_on(db.insert_project(&wscope(p.project.workspace_id.0.as_u128(), 1, Role::Admin), p)).unwrap();
}

// --- making a project --------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clash {
    Nothing,
    Id,
    Identifier,
    Name,
    /// The same name and key in the other workspace: allowed.
    OtherWorkspace,
    CreatorIsNoUser,
    LeadIsNoUser,
    MemberIsNoUser,
    /// A user who exists and is a member of no workspace: Postgres's `project_members_workspace_member_fkey` refuses them.
    MemberIsNoMemberOfTheWorkspace,
    StateNameTwice,
    TwoDefaultStates,
}

/// A creation either makes the project, its members, their settings rows and its states, or does nothing: every way it can fail
/// leaves no row behind, and the project that was there is as it was. A taken id, key or name is a `Conflict`; a user who does
/// not exist is a `Backend` error, as a foreign key is in Postgres.
#[rstest]
fn a_creation_makes_the_project_its_members_and_its_states_together_or_not_at_all(
    #[values(
        Clash::Nothing,
        Clash::Id,
        Clash::Identifier,
        Clash::Name,
        Clash::OtherWorkspace,
        Clash::CreatorIsNoUser,
        Clash::LeadIsNoUser,
        Clash::MemberIsNoUser,
        Clash::MemberIsNoMemberOfTheWorkspace,
        Clash::StateNameTwice,
        Clash::TwoDefaultStates
    )]
    clash: Clash,
) {
    let db = world();
    let first = new_project(A, 10, 2, &[], 0);
    make(&db, &first);

    let mut second = new_project(A, 11, 3, &[], 1);
    match clash {
        Clash::Nothing => {}
        Clash::Id => second.project.id = first.project.id,
        Clash::Identifier => second.project.identifier = first.project.identifier.clone(),
        Clash::Name => second.project.name = first.project.name.clone(),
        Clash::OtherWorkspace => {
            second = new_project(B, 11, 3, &[], 1);
            second.project.name = first.project.name.clone();
            second.project.identifier = first.project.identifier.clone();
        }
        Clash::CreatorIsNoUser => second.project.created_by = Some(uid(99)),
        Clash::LeadIsNoUser => second.project.project_lead = Some(uid(99)),
        Clash::MemberIsNoUser => second.members[0].member.user_id = uid(99),
        Clash::MemberIsNoMemberOfTheWorkspace => {
            block_on(db.seed(&sys(), &SeedFixture { users: vec![uid(9)], ..SeedFixture::default() })).unwrap();
            second.members[0].member.user_id = uid(9);
        }
        Clash::StateNameTwice => second.states[1].name = second.states[0].name.clone(),
        Clash::TwoDefaultStates => second.states[1].is_default = true,
    }
    let ws = second.project.workspace_id.0.as_u128();
    let got = block_on(db.insert_project(&wscope(ws, 1, Role::Admin), &second));
    match clash {
        Clash::Nothing | Clash::OtherWorkspace => assert!(got.is_ok(), "{clash:?}: {got:?}"),
        Clash::Id | Clash::Identifier | Clash::Name | Clash::StateNameTwice | Clash::TwoDefaultStates => {
            assert!(matches!(got, Err(DbError::Conflict(_))), "{clash:?}: {got:?}")
        }
        Clash::CreatorIsNoUser | Clash::LeadIsNoUser | Clash::MemberIsNoUser | Clash::MemberIsNoMemberOfTheWorkspace => {
            assert!(matches!(got, Err(DbError::Backend(_))), "{clash:?}: {got:?}")
        }
    }
    let made = block_on(db.project_detail(&wscope(ws, 3, Role::Admin), second.project.id)).unwrap();
    let states = block_on(db.project_states(&pscope(ws, 11, 3))).unwrap();
    if got.is_ok() {
        assert!(made.is_some(), "{clash:?}: the project is not there");
        assert_eq!(states.len(), 5, "{clash:?}");
    } else if clash != Clash::Id {
        // nothing of the refused project is left: not the project, not its members' rows, not its states
        assert!(made.is_none(), "{clash:?}: a refused project is there");
        assert!(states.is_empty(), "{clash:?}: its states are there");
        assert_eq!(block_on(db.project_members(&pscope(ws, 11, 3))).unwrap(), vec![], "{clash:?}: its members are there");
    }
    // the first project is as it was
    let original = block_on(db.project_detail(&wscope(A, 2, Role::Admin), first.project.id)).unwrap().unwrap();
    assert_eq!(original.listing.project, first.project, "{clash:?}");
    assert_eq!(block_on(db.project_states(&pscope(A, 10, 2))).unwrap().len(), 5);
}

/// A member's place in their own list is 10000 below the lowest they hold in the workspace, or 65535 when they hold none, and each
/// person's places are their own, and each workspace's.
#[test]
fn a_members_place_is_ten_thousand_below_the_lowest_they_hold_in_the_workspace() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[3], 0));
    make(&db, &new_project(A, 11, 2, &[], 1));
    make(&db, &new_project(A, 12, 3, &[2], 2));
    make(&db, &new_project(B, 13, 2, &[], 3));
    let place = |ws: u128, project: u128, user: u128| {
        block_on(db.project_detail(&wscope(ws, user, Role::Member), pid(project))).unwrap().unwrap().listing.sort_order
    };
    // user 2: 65535, then 55535, then 45535 in workspace A, and 65535 again in B; user 3: 65535, then (not in 11) 55535 in 12
    assert_eq!([place(A, 10, 2), place(A, 11, 2), place(A, 12, 2), place(B, 13, 2)], [Some(65535.0), Some(55535.0), Some(45535.0), Some(65535.0)]);
    assert_eq!([place(A, 10, 3), place(A, 11, 3), place(A, 12, 3)], [Some(65535.0), None, Some(55535.0)]);
}

proptest! {
    #![proptest_config(proptest_config(128))]

    /// Whoever makes projects in whichever workspace, each member's place in each new project is 10000 below the lowest place they
    /// hold in that workspace so far (their first is 65535), whatever the order of the creations.
    #[test]
    fn prop_a_new_projects_places_follow_the_lowest_each_member_holds(
        creations in prop::collection::vec((prop_oneof![Just(A), Just(B)], 2..5u128, prop::option::of(2..5u128)), 1..10),
    ) {
        let db = world();
        let mut lowest: std::collections::BTreeMap<(u128, u128), f64> = Default::default();
        for (i, (ws, creator, other)) in creations.into_iter().enumerate() {
            let n = 100 + i as u128;
            let others: Vec<u128> = other.filter(|o| *o != creator).into_iter().collect();
            make(&db, &new_project(ws, n, creator, &others, i as i64));
            for user in std::iter::once(creator).chain(others) {
                let want = lowest.get(&(ws, user)).map_or(65535.0, |l| l - 10000.0);
                lowest.insert((ws, user), want);
                let got = block_on(db.project_detail(&wscope(ws, user, Role::Member), pid(n))).unwrap().unwrap().listing.sort_order;
                prop_assert_eq!(got, Some(want));
            }
        }
    }
}

// --- clashes and changes -----------------------------------------------------------------------------------------------------

#[rstest]
#[case::the_name("p10", None, None, true, false)]
#[case::the_key("zzz", Some("P10"), None, false, true)]
#[case::both("p10", Some("P10"), None, true, true)]
#[case::another_case_of_the_name("P10", None, None, false, false)]
#[case::another_case_of_the_key("zzz", Some("p10"), None, false, false)]
#[case::the_project_itself_is_left_out("p10", Some("P10"), Some(10), false, false)]
#[case::another_project_is_not_left_out("p10", Some("P10"), Some(11), true, true)]
#[case::neither_is_asked("zzz", None, None, false, false)]
fn a_name_or_key_clashes_only_when_it_is_exactly_another_projects(
    #[case] name: &str,
    #[case] identifier: Option<&str>,
    #[case] except: Option<u128>,
    #[case] name_clash: bool,
    #[case] identifier_clash: bool,
) {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    let got = block_on(db.project_clashes(&wscope(A, 2, Role::Admin), Some(name), identifier, except.map(pid))).unwrap();
    assert_eq!(got, ProjectClashes { name: name_clash, identifier: identifier_clash });
    // the other workspace has no such project
    let other = block_on(db.project_clashes(&wscope(B, 2, Role::Admin), Some(name), identifier, None)).unwrap();
    assert_eq!(other, ProjectClashes::default());
}

#[test]
fn an_update_replaces_what_can_change_and_keeps_the_id_the_workspace_and_the_creation() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    make(&db, &new_project(A, 11, 3, &[], 1));
    let scope = wscope(A, 2, Role::Admin);
    let mut row = block_on(db.project_detail(&scope, pid(10))).unwrap().unwrap().listing.project;
    row.name = "renamed".into();
    row.identifier = "NEW".into();
    row.network = ProjectNetwork::Secret;
    row.description = "d".into();
    row.updated_by = Some(uid(3));
    row.updated_at = at(50);
    // what it must not change
    row.created_at = at(999);
    row.created_by = Some(uid(7));
    row.workspace_id = wid(B);
    assert!(block_on(db.update_project(&scope, &row)).unwrap());
    let got = block_on(db.project_detail(&scope, pid(10))).unwrap().unwrap().listing.project;
    assert_eq!((got.name.as_str(), got.identifier.as_str(), got.network, got.description.as_str()), ("renamed", "NEW", ProjectNetwork::Secret, "d"));
    assert_eq!((got.updated_by, got.updated_at), (Some(uid(3)), at(50)));
    assert_eq!((got.created_at, got.created_by, got.workspace_id), (at(0), Some(uid(2)), wid(A)));
    // its own name and key are not a clash, another project's are
    let mut again = got.clone();
    again.description = "e".into();
    assert!(block_on(db.update_project(&scope, &again)).unwrap());
    let mut taken = got.clone();
    taken.name = "p11".into();
    assert!(matches!(block_on(db.update_project(&scope, &taken)), Err(DbError::Conflict(_))));
    let mut taken = got;
    taken.identifier = "P11".into();
    assert!(matches!(block_on(db.update_project(&scope, &taken)), Err(DbError::Conflict(_))));
    // a project that is not in the scope's workspace is not there, and a user who does not exist is refused
    let mut unknown = again.clone();
    unknown.id = pid(77);
    assert!(!block_on(db.update_project(&scope, &unknown)).unwrap());
    assert!(!block_on(db.update_project(&wscope(B, 2, Role::Admin), &again)).unwrap());
    let mut lead = again;
    lead.project_lead = Some(uid(99));
    assert!(matches!(block_on(db.update_project(&scope, &lead)), Err(DbError::Backend(_))));
}

// --- who is in a project -----------------------------------------------------------------------------------------------------

#[test]
fn a_project_says_the_callers_role_who_is_in_it_newest_first_and_the_next_number() {
    let db = world();
    // 2 made it, 3 and 4 came with it (a microsecond apart); a stray row of 5 names the other workspace
    make(&db, &new_project(A, 10, 2, &[3, 4], 0));
    block_on(db.seed(
        &sys(),
        &SeedFixture {
            memberships: vec![SeedMembership { project_id: pid(10), workspace_id: wid(B), user_id: uid(5), role: Role::Admin }],
            project_sort_orders: vec![SeedProjectSortOrder { project_id: pid(10), user_id: uid(5), sort_order: 7.0 }],
            ..SeedFixture::default()
        },
    ))
    .unwrap();
    let d = |user: u128| block_on(db.project_detail(&wscope(A, user, Role::Member), pid(10))).unwrap().unwrap();
    // newest row first: the last to join is listed first
    assert_eq!(d(2).members, vec![uid(4), uid(3), uid(2)]);
    assert_eq!((d(2).listing.role, d(4).listing.role, d(6).listing.role), (Some(Role::Admin), Some(Role::Admin), None));
    // a stray row is no role and no member, though the place of its owner is theirs
    assert_eq!((d(5).listing.role, d(5).listing.sort_order), (None, Some(7.0)));
    assert!(!d(2).members.contains(&uid(5)));
    // no such project, and a project of the other workspace
    assert!(block_on(db.project_detail(&wscope(A, 2, Role::Admin), pid(77))).unwrap().is_none());
    assert!(block_on(db.project_detail(&wscope(B, 2, Role::Admin), pid(10))).unwrap().is_none());
    // the number the next work item will have: one more than the last handed out
    assert_eq!(d(2).next_sequence, 1);
    let scope = pscope(A, 10, 2);
    let mut tx = block_on(db.begin(&scope)).unwrap();
    block_on(tx.allocate(&scope)).unwrap();
    block_on(tx.allocate(&scope)).unwrap();
    block_on(tx.commit()).unwrap();
    assert_eq!(d(2).next_sequence, 3);
}

#[test]
fn a_members_own_row_names_the_person_the_project_and_the_workspace() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    block_on(db.seed(
        &sys(),
        &SeedFixture { memberships: vec![SeedMembership { project_id: pid(10), workspace_id: wid(B), user_id: uid(5), role: Role::Admin }], ..SeedFixture::default() },
    ))
    .unwrap();
    let got = block_on(db.project_member(&wscope(A, 2, Role::Member), pid(10))).unwrap().unwrap();
    assert_eq!((got.member.user_id, got.member.role, got.user.id), (uid(2), Role::Admin, uid(2)));
    assert_eq!((got.project.id, got.workspace.id, got.workspace.slug.as_str()), (pid(10), wid(A), "a"));
    // nobody else's row, and no row for a person who has none, a stray one, a project of another workspace or one that is not there
    assert!(block_on(db.project_member(&wscope(A, 6, Role::Member), pid(10))).unwrap().is_none());
    assert!(block_on(db.project_member(&wscope(A, 5, Role::Member), pid(10))).unwrap().is_none());
    assert!(block_on(db.project_member(&wscope(B, 2, Role::Member), pid(10))).unwrap().is_none());
    assert!(block_on(db.project_member(&wscope(A, 2, Role::Member), pid(77))).unwrap().is_none());
}

#[test]
fn the_member_list_leaves_out_people_who_left_the_workspace_and_stray_rows() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[3, 4], 0));
    block_on(db.seed(
        &sys(),
        &SeedFixture { memberships: vec![SeedMembership { project_id: pid(10), workspace_id: wid(B), user_id: uid(5), role: Role::Member }], ..SeedFixture::default() },
    ))
    .unwrap();
    let ids = || block_on(db.project_members(&pscope(A, 10, 2))).unwrap().iter().map(|m| m.user_id).collect::<Vec<_>>();
    assert_eq!(ids(), vec![uid(4), uid(3), uid(2)]);
    assert!(block_on(db.set_workspace_member_active(&sys(), wid(A), uid(3), false)).unwrap());
    assert_eq!(ids(), vec![uid(4), uid(2)], "a person who left the workspace is no longer listed");
    assert!(block_on(db.set_workspace_member_active(&sys(), wid(A), uid(3), true)).unwrap());
    assert_eq!(ids(), vec![uid(4), uid(3), uid(2)]);
}

#[test]
fn a_members_settings_are_made_once_and_are_theirs() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    let props = |salt: i64, user: u128| ProjectUserProperties {
        id: Uuid::from_u128(0x9000 + salt as u128),
        project_id: pid(10),
        workspace_id: wid(A),
        user_id: uid(user),
        filters: json!({"salt": salt}),
        display_filters: json!({}),
        display_properties: json!({}),
        rich_filters: json!({}),
        preferences: json!({}),
        sort_order: 5.0,
        created_at: at(salt),
        updated_at: at(salt),
    };
    // the creator has theirs already (made with the project) and asking again changes nothing
    let mine = block_on(db.ensure_project_user_properties(&pscope(A, 10, 2), &props(1, 2))).unwrap();
    assert_eq!(mine.sort_order, 65535.0);
    assert_eq!(block_on(db.ensure_project_user_properties(&pscope(A, 10, 2), &props(2, 2))).unwrap(), mine);
    // another member has none yet: theirs is made from what is asked, and then kept
    let first = block_on(db.ensure_project_user_properties(&pscope(A, 10, 6), &props(3, 6))).unwrap();
    assert_eq!((first.id, first.sort_order, first.user_id), (Uuid::from_u128(0x9003), 5.0, uid(6)));
    assert_eq!(block_on(db.ensure_project_user_properties(&pscope(A, 10, 6), &props(4, 6))).unwrap(), first);
    // the row is the scope's, whatever project, workspace and person the new one names (the Postgres insert takes them from the
    // scope and the project's own row too)
    let stray = ProjectUserProperties { project_id: pid(11), workspace_id: wid(B), user_id: uid(8), ..props(6, 6) };
    let theirs = block_on(db.ensure_project_user_properties(&pscope(A, 10, 7), &stray)).unwrap();
    assert_eq!((theirs.project_id, theirs.workspace_id, theirs.user_id, theirs.id), (pid(10), wid(A), uid(7), Uuid::from_u128(0x9006)));
    // a project that is not in the scope's workspace has none to make
    assert!(matches!(block_on(db.ensure_project_user_properties(&pscope(B, 10, 6), &props(5, 6))), Err(DbError::Backend(_))));
}

/// `is_workspace_member` is about anyone, not the scope's own user: an active member is one, a member who has left is not, nor is a
/// user who has no row in the workspace, nor one of another workspace's.
#[test]
fn anyone_is_asked_whether_they_are_an_active_member_of_the_workspace() {
    let db = world();
    let asked = |ws: u128, user: u128| block_on(db.is_workspace_member(&wscope(ws, 1, Role::Admin), uid(user))).unwrap();
    assert!(asked(A, 1) && asked(A, 5) && asked(B, 5));
    assert!(block_on(db.set_workspace_member_active(&sys(), wid(A), uid(5), false)).unwrap());
    assert!(!asked(A, 5), "a member who has left is no member");
    assert!(asked(B, 5), "and is one still of the other workspace");
    block_on(db.seed(&sys(), &SeedFixture { users: vec![uid(9)], ..SeedFixture::default() })).unwrap();
    assert!(!asked(A, 9) && !asked(B, 9), "a user with no row in the workspace is no member");
    assert!(!asked(A, 99), "nor is a user who does not exist");
    assert!(block_on(db.set_workspace_member_active(&sys(), wid(A), uid(5), true)).unwrap());
    assert!(asked(A, 5));
}

proptest! {
    #![proptest_config(proptest_config(128))]

    /// A seeded project member and the settings row seeded for them each have an id of their own, made from the project and the
    /// person: no two share one, whatever projects and people they are of (Postgres keeps a unique index on each, and the app names a
    /// member by theirs).
    #[test]
    fn prop_seeded_project_members_and_their_settings_have_ids_of_their_own(
        pairs in prop::collection::btree_set((1..5u128, 1..6u128), 1..12),
    ) {
        let db = world();
        // Project ids with every high bit set (the complement of a small number) and people with small ones: the two overlap, as
        // random ids do, so a combination of the two that only works while they do not (a `|` or an `&` for the `^`) gives
        // different pairs one id. The domain is one where the real combination is injective: it is not on every pair of ids (two
        // projects that are each other's half-swaps with two people who are would meet), which random ids make improbable.
        let (project, user) = (|p: u128| pid(!p), uid);
        let projects: std::collections::BTreeSet<u128> = pairs.iter().map(|p| p.0).collect();
        let f = SeedFixture {
            projects: projects.iter().map(|p| SeedProject { id: project(*p), workspace_id: wid(A), name: format!("p{p}") }).collect(),
            memberships: pairs.iter().map(|(p, u)| SeedMembership { project_id: project(*p), workspace_id: wid(A), user_id: user(*u), role: Role::Member }).collect(),
            project_sort_orders: pairs.iter().map(|(p, u)| SeedProjectSortOrder { project_id: project(*p), user_id: user(*u), sort_order: 1.0 }).collect(),
            ..SeedFixture::default()
        };
        block_on(db.seed(&sys(), &f)).unwrap();
        let mut member_ids = std::collections::BTreeSet::new();
        let mut settings_ids = std::collections::BTreeSet::new();
        for (p, u) in &pairs {
            let caller = WorkspaceScope::mint(wid(A), user(*u), Role::Member);
            let mine = block_on(db.project_member(&caller, project(*p))).unwrap().expect("a seeded member has a row");
            member_ids.insert(mine.member.id);
            let scope = Scope::mint(wid(A), project(*p), user(*u));
            let theirs = block_on(db.ensure_project_user_properties(&scope, &stray_settings(project(*p), user(*u)))).unwrap();
            prop_assert_eq!(theirs.sort_order, 1.0, "the seeded row is the one asked for");
            settings_ids.insert(theirs.id);
        }
        prop_assert_eq!(member_ids.len(), pairs.len(), "two seeded project members share an id");
        prop_assert_eq!(settings_ids.len(), pairs.len(), "two seeded settings rows share an id");
    }
}

/// Settings that a person who has none could be given: a seeded row is kept, and shows by its sort order of 1.
fn stray_settings(project: ProjectId, user: UserId) -> ProjectUserProperties {
    ProjectUserProperties {
        id: Uuid::nil(),
        project_id: project,
        workspace_id: wid(A),
        user_id: user,
        filters: json!({}),
        display_filters: json!({}),
        display_properties: json!({}),
        rich_filters: json!({}),
        preferences: json!({}),
        sort_order: 99.0,
        created_at: at(0),
        updated_at: at(0),
    }
}

// --- states, labels, counts --------------------------------------------------------------------------------------------------

#[test]
fn the_state_lists_leave_out_the_intake_state_and_follow_sequence() {
    let db = world();
    let mut p = new_project(A, 10, 2, &[], 0);
    // out of order in the table, and two with one sequence: the id breaks the tie
    p.states.reverse();
    // index 0 is now the intake state; index 1 (Cancelled) shares Todo's sequence
    p.states[1].sequence = 25000.0;
    make(&db, &p);
    let names: Vec<(String, f64)> = block_on(db.project_states(&pscope(A, 10, 2))).unwrap().iter().map(|s| (s.name.clone(), s.sequence)).collect();
    assert!(names.windows(2).all(|w| w[0].1 <= w[1].1), "{names:?}");
    assert_eq!(names.len(), 5);
    assert!(names.iter().all(|(n, _)| n != "Triage"));
    let tie: Vec<Uuid> = block_on(db.project_states(&pscope(A, 10, 2))).unwrap().iter().filter(|s| s.sequence == 25000.0).map(|s| s.id).collect();
    assert_eq!(tie.len(), 2);
    assert!(tie[0] < tie[1], "ties are broken by id");
    // the intake state is its own
    let triage = block_on(db.triage_state(&pscope(A, 10, 2))).unwrap().unwrap();
    assert_eq!((triage.name.as_str(), triage.group, triage.sequence), ("Triage", StateGroup::Triage, 65000.0));
    assert!(block_on(db.triage_state(&pscope(A, 77, 2))).unwrap().is_none());
    // another project's states are not these
    make(&db, &new_project(A, 11, 3, &[], 1));
    assert_eq!(block_on(db.project_states(&pscope(A, 10, 2))).unwrap().len(), 5);
}

#[test]
fn the_workspaces_states_are_those_of_the_projects_the_caller_is_in_that_are_not_archived() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[3], 0));
    make(&db, &new_project(A, 11, 3, &[], 1));
    make(&db, &new_project(A, 12, 2, &[], 2));
    make(&db, &new_project(B, 13, 2, &[], 3));
    let projects_of = |user: u128| -> Vec<Uuid> {
        let all: std::collections::BTreeSet<Uuid> =
            block_on(db.workspace_states(&wscope(A, user, Role::Member))).unwrap().iter().map(|s| s.project_id.0).collect();
        all.into_iter().collect()
    };
    // 2 is in 10 and 12, 3 in 10 and 11, 4 in none; every state of those projects, five each, and none of workspace B's
    assert_eq!(block_on(db.workspace_states(&wscope(A, 2, Role::Member))).unwrap().len(), 10);
    assert_eq!(projects_of(2), vec![pid(10).0, pid(12).0]);
    assert_eq!(projects_of(3), vec![pid(10).0, pid(11).0]);
    assert!(projects_of(4).is_empty());
    // by sequence, projects with one sequence side by side
    let seqs: Vec<f64> = block_on(db.workspace_states(&wscope(A, 2, Role::Member))).unwrap().iter().map(|s| s.sequence).collect();
    assert!(seqs.windows(2).all(|w| w[0] <= w[1]), "{seqs:?}");
    // an archived project's states are not listed
    let mut row = block_on(db.project_detail(&wscope(A, 2, Role::Admin), pid(12))).unwrap().unwrap().listing.project;
    row.archived_at = Some(at(60));
    assert!(block_on(db.update_project(&wscope(A, 2, Role::Admin), &row)).unwrap());
    assert_eq!(projects_of(2), vec![pid(10).0]);
}

#[test]
fn a_projects_labels_are_its_own_by_sort_order_then_id() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    make(&db, &new_project(A, 11, 2, &[], 1));
    let label = |n: u128, project: u128, sort_order: f64| SeedLabel {
        id: Uuid::from_u128(n),
        project_id: pid(project),
        workspace_id: wid(A),
        name: format!("l{n}"),
        color: "#fff".into(),
        parent_id: None,
        sort_order,
    };
    block_on(db.seed(&sys(), &SeedFixture { labels: vec![label(5, 10, 3.0), label(4, 10, 1.0), label(3, 10, 3.0), label(9, 11, 0.5)], ..SeedFixture::default() })).unwrap();
    let ids: Vec<u128> = block_on(db.project_labels(&pscope(A, 10, 2))).unwrap().iter().map(|l| l.id.as_u128()).collect();
    assert_eq!(ids, vec![4, 3, 5]);
    assert_eq!(block_on(db.project_labels(&pscope(A, 11, 2))).unwrap().len(), 1);
    assert!(block_on(db.project_labels(&pscope(B, 10, 2))).unwrap().is_empty());
}

#[test]
fn the_workspaces_counts_cover_every_project_whoever_asks_newest_first() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[3], 0));
    make(&db, &new_project(A, 11, 3, &[], 5));
    make(&db, &new_project(B, 13, 2, &[], 3));
    // two work items in project 10, one of them a draft; one in project 11, archived
    let item = |n: u128, project: u128, draft: bool, archived: bool| WorkItem {
        id: WorkItemId(Uuid::from_u128(n)),
        workspace_id: wid(A),
        project_id: pid(project),
        sequence_id: n as i64,
        sort_order: 1,
        name: format!("w{n}"),
        description: None,
        external_source: None,
        external_id: None,
        created_by: uid(2),
        created_at: at(0),
        archived_at: archived.then(|| at(1)),
        is_draft: draft,
        deleted_at: None,
    };
    for (n, project, draft, archived) in [(1, 10, false, false), (2, 10, true, false), (3, 10, false, false), (4, 11, false, true)] {
        let scope = pscope(A, project, 2);
        let mut tx = block_on(db.begin(&scope)).unwrap();
        block_on(tx.insert_work_item(&scope, &item(n, project, draft, archived))).unwrap();
        block_on(tx.commit()).unwrap();
    }
    // a person who is in neither project, and a person who is in one of them, are told the same
    for user in [6u128, 2, 3] {
        let got: Vec<(ProjectId, i64, i64)> =
            block_on(db.project_stats(&wscope(A, user, Role::Guest), None)).unwrap().iter().map(|s| (s.id, s.total_issues, s.total_members)).collect();
        assert_eq!(got, vec![(pid(11), 0, 1), (pid(10), 2, 2)], "user {user}");
    }
    // only the projects asked for, and one of another workspace is not there
    let got = block_on(db.project_stats(&wscope(A, 2, Role::Member), Some(&[pid(10), pid(13)]))).unwrap();
    assert_eq!(got.iter().map(|s| s.id).collect::<Vec<_>>(), vec![pid(10)]);
    assert!(block_on(db.project_stats(&wscope(A, 2, Role::Member), Some(&[]))).unwrap().is_empty());
}
