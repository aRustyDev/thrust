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
        workspace_members: [A, B]
            .iter()
            .flat_map(|w| (2..=8).map(move |u| SeedWorkspaceMember { workspace_id: wid(*w), user_id: uid(u) }))
            .collect(),
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
        external_source: None,
        external_id: None,
        updated_by: None,
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
        Clash::CreatorIsNoUser
        | Clash::LeadIsNoUser
        | Clash::MemberIsNoUser
        | Clash::MemberIsNoMemberOfTheWorkspace => {
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
        assert_eq!(
            block_on(db.project_members(&pscope(ws, 11, 3))).unwrap(),
            vec![],
            "{clash:?}: its members are there"
        );
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
    assert_eq!(
        [place(A, 10, 2), place(A, 11, 2), place(A, 12, 2), place(B, 13, 2)],
        [Some(65535.0), Some(55535.0), Some(45535.0), Some(65535.0)]
    );
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
    let got =
        block_on(db.project_clashes(&wscope(A, 2, Role::Admin), Some(name), identifier, except.map(pid))).unwrap();
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
    assert_eq!(
        (got.name.as_str(), got.identifier.as_str(), got.network, got.description.as_str()),
        ("renamed", "NEW", ProjectNetwork::Secret, "d")
    );
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
            memberships: vec![SeedMembership {
                project_id: pid(10),
                workspace_id: wid(B),
                user_id: uid(5),
                role: Role::Admin,
            }],
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
        &SeedFixture {
            memberships: vec![SeedMembership {
                project_id: pid(10),
                workspace_id: wid(B),
                user_id: uid(5),
                role: Role::Admin,
            }],
            ..SeedFixture::default()
        },
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
        &SeedFixture {
            memberships: vec![SeedMembership {
                project_id: pid(10),
                workspace_id: wid(B),
                user_id: uid(5),
                role: Role::Member,
            }],
            ..SeedFixture::default()
        },
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
    assert_eq!(
        (theirs.project_id, theirs.workspace_id, theirs.user_id, theirs.id),
        (pid(10), wid(A), uid(7), Uuid::from_u128(0x9006))
    );
    // a project that is not in the scope's workspace has none to make
    assert!(matches!(
        block_on(db.ensure_project_user_properties(&pscope(B, 10, 6), &props(5, 6))),
        Err(DbError::Backend(_))
    ));
}

/// `is_workspace_member` is about anyone, not the scope's own user: an active member is one, a member who has left is not, nor is a
/// user who has no row in the workspace, nor one of another workspace's.
#[test]
fn anyone_is_asked_whether_they_are_an_active_member_of_the_workspace() {
    let db = world();
    let asked =
        |ws: u128, user: u128| block_on(db.is_workspace_member(&wscope(ws, 1, Role::Admin), uid(user))).unwrap();
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
    let names: Vec<(String, f64)> =
        block_on(db.project_states(&pscope(A, 10, 2))).unwrap().iter().map(|s| (s.name.clone(), s.sequence)).collect();
    assert!(names.windows(2).all(|w| w[0].1 <= w[1].1), "{names:?}");
    assert_eq!(names.len(), 5);
    assert!(names.iter().all(|(n, _)| n != "Triage"));
    let tie: Vec<Uuid> = block_on(db.project_states(&pscope(A, 10, 2)))
        .unwrap()
        .iter()
        .filter(|s| s.sequence == 25000.0)
        .map(|s| s.id)
        .collect();
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
        let all: std::collections::BTreeSet<Uuid> = block_on(db.workspace_states(&wscope(A, user, Role::Member)))
            .unwrap()
            .iter()
            .map(|s| s.project_id.0)
            .collect();
        all.into_iter().collect()
    };
    // 2 is in 10 and 12, 3 in 10 and 11, 4 in none; every state of those projects, five each, and none of workspace B's
    assert_eq!(block_on(db.workspace_states(&wscope(A, 2, Role::Member))).unwrap().len(), 10);
    assert_eq!(projects_of(2), vec![pid(10).0, pid(12).0]);
    assert_eq!(projects_of(3), vec![pid(10).0, pid(11).0]);
    assert!(projects_of(4).is_empty());
    // by sequence, projects with one sequence side by side
    let seqs: Vec<f64> =
        block_on(db.workspace_states(&wscope(A, 2, Role::Member))).unwrap().iter().map(|s| s.sequence).collect();
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
    block_on(db.seed(
        &sys(),
        &SeedFixture {
            labels: vec![label(5, 10, 3.0), label(4, 10, 1.0), label(3, 10, 3.0), label(9, 11, 0.5)],
            ..SeedFixture::default()
        },
    ))
    .unwrap();
    let ids: Vec<u128> =
        block_on(db.project_labels(&pscope(A, 10, 2))).unwrap().iter().map(|l| l.id.as_u128()).collect();
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
        created_by: Some(uid(2)),
        created_at: at(0),
        archived_at: archived.then(|| at(1)),
        is_draft: draft,
        deleted_at: None,
    };
    for (n, project, draft, archived) in
        [(1, 10, false, false), (2, 10, true, false), (3, 10, false, false), (4, 11, false, true)]
    {
        let scope = pscope(A, project, 2);
        let mut tx = block_on(db.begin(&scope)).unwrap();
        block_on(tx.insert_work_item(&scope, &item(n, project, draft, archived))).unwrap();
        block_on(tx.commit()).unwrap();
    }
    // a person who is in neither project, and a person who is in one of them, are told the same
    for user in [6u128, 2, 3] {
        let got: Vec<(ProjectId, i64, i64)> = block_on(db.project_stats(&wscope(A, user, Role::Guest), None))
            .unwrap()
            .iter()
            .map(|s| (s.id, s.total_issues, s.total_members))
            .collect();
        assert_eq!(got, vec![(pid(11), 0, 1), (pid(10), 2, 2)], "user {user}");
    }
    // only the projects asked for, and one of another workspace is not there
    let got = block_on(db.project_stats(&wscope(A, 2, Role::Member), Some(&[pid(10), pid(13)]))).unwrap();
    assert_eq!(got.iter().map(|s| s.id).collect::<Vec<_>>(), vec![pid(10)]);
    assert!(block_on(db.project_stats(&wscope(A, 2, Role::Member), Some(&[]))).unwrap().is_empty());
}

// --- states (issue #58, migration 0015) ---------------------------------------------------------------------------------------
//
// The rules, written out:
//
// * `insert_state` gives a state Plane's place: after the greatest of the project's other live states that are not the intake one,
//   by 15000, whatever place the row asked for; the first such state keeps the place it asked for. A state that is the default takes
//   the default from the others. A name a live state of the project has is a conflict; one a deleted state had, or another project's
//   state has, is not.
// * `update_state` replaces name, description, colour, place, group, default flag, external key, editor and time, keeps the rest, and
//   finds only a live state of the scope's project; a state that becomes the default takes it from the others, and a change that finds
//   no state changes nothing.
// * `delete_state` marks the state: the three lists leave it out and its name is free, while a work item that names it still reads its
//   group. `state_has_work_items` counts what the pin's `Issue.objects` counts: not a draft, not archived, not deleted, and nothing in
//   a project that is archived.

fn state_id(project: u128, k: u128) -> Uuid {
    Uuid::from_u128((project << 48) | (0x100 + k))
}

/// A state of project `project`, made by user 2, with the place `sequence` (which `insert_state` replaces unless it is the first).
fn new_state(project: u128, k: u128, name: &str, sequence: f64) -> StateRow {
    StateRow {
        id: state_id(project, k),
        workspace_id: wid(A),
        project_id: pid(project),
        name: name.into(),
        description: String::new(),
        color: "#123456".into(),
        sequence,
        group: StateGroup::Started,
        is_default: false,
        created_by: Some(uid(2)),
        created_at: at(10),
        updated_at: at(10),
        external_source: None,
        external_id: None,
        updated_by: None,
    }
}

fn states(db: &MemoryDatabase, project: u128) -> Vec<StateRow> {
    block_on(db.project_states(&pscope(A, project, 2))).unwrap()
}

fn named(db: &MemoryDatabase, project: u128, name: &str) -> StateRow {
    states(db, project).into_iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no state {name}"))
}

fn defaults(db: &MemoryDatabase, project: u128) -> Vec<String> {
    states(db, project).into_iter().filter(|s| s.is_default).map(|s| s.name).collect()
}

/// A work item `n` of project 10 in `state`, written with its attributes in one transaction.
fn work_item_in(db: &MemoryDatabase, n: u128, state: Uuid, draft: bool, archived: bool, deleted: bool) -> WorkItem {
    let item = WorkItem {
        id: WorkItemId(Uuid::from_u128(n)),
        workspace_id: wid(A),
        project_id: pid(10),
        sequence_id: n as i64,
        sort_order: 1,
        name: format!("w{n}"),
        description: None,
        external_source: None,
        external_id: None,
        created_by: Some(uid(2)),
        created_at: at(0),
        archived_at: archived.then(|| at(1)),
        is_draft: draft,
        deleted_at: deleted.then(|| at(1)),
    };
    let scope = pscope(A, 10, 2);
    let mut tx = block_on(db.begin(&scope)).unwrap();
    block_on(tx.insert_work_item(&scope, &item)).unwrap();
    let attrs = WorkItemAttrs { state_id: Some(state), ..WorkItemAttrs::bare(at(0)) };
    block_on(tx.insert_work_item_attrs(&scope, item.id, &attrs)).unwrap();
    block_on(tx.commit()).unwrap();
    item
}

#[test]
fn a_new_state_goes_after_the_greatest_live_one_and_the_intake_state_does_not_count() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[3], 0));
    let scope = pscope(A, 10, 2);
    // the project has 15000 to 55000, and an intake state at 65000 that does not count: 55000 + 15000
    let a = block_on(db.insert_state(&scope, &new_state(10, 1, "Review", 1.0))).unwrap();
    assert_eq!(a.sequence, 70000.0);
    // what is returned is what is stored
    assert_eq!(named(&db, 10, "Review"), a);
    // and the next goes after that one, whatever it asked for
    assert_eq!(block_on(db.insert_state(&scope, &new_state(10, 2, "QA", 9e9))).unwrap().sequence, 85000.0);
    let last = block_on(db.insert_state(&scope, &new_state(10, 3, "Ops", 0.0))).unwrap();
    assert_eq!(last.sequence, 100_000.0);
    // a deleted state is not counted: the next goes where the deleted one went
    assert!(block_on(db.delete_state(&scope, last.id, at(11))).unwrap());
    assert_eq!(block_on(db.insert_state(&scope, &new_state(10, 4, "Ops2", 0.0))).unwrap().sequence, 100_000.0);
    // another project's states do not count
    make(&db, &new_project(A, 11, 2, &[], 1));
    assert_eq!(
        block_on(db.insert_state(&pscope(A, 11, 2), &new_state(11, 1, "Review", 0.0))).unwrap().sequence,
        70000.0
    );
}

#[test]
fn the_first_state_of_a_project_keeps_the_place_it_asked_for() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    let scope = pscope(A, 10, 2);
    for s in states(&db, 10) {
        assert!(block_on(db.delete_state(&scope, s.id, at(11))).unwrap(), "{}", s.name);
    }
    // only the intake state is left, and it is not one that counts
    let first = block_on(db.insert_state(&scope, &new_state(10, 1, "Only", 777.0))).unwrap();
    assert_eq!(first.sequence, 777.0);
    assert_eq!(block_on(db.insert_state(&scope, &new_state(10, 2, "Next", 5.0))).unwrap().sequence, 15_777.0);
}

#[test]
fn a_state_that_becomes_the_default_takes_it_from_the_others_and_a_change_that_finds_nothing_changes_nothing() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    let scope = pscope(A, 10, 2);
    assert_eq!(defaults(&db, 10), ["Backlog"]);
    // a state that is not the default leaves it where it is
    block_on(db.insert_state(&scope, &new_state(10, 1, "Plain", 0.0))).unwrap();
    assert_eq!(defaults(&db, 10), ["Backlog"]);
    // a new one that is, takes it
    let review = StateRow { is_default: true, ..new_state(10, 2, "Review", 0.0) };
    block_on(db.insert_state(&scope, &review)).unwrap();
    assert_eq!(defaults(&db, 10), ["Review"]);
    // a change that makes another the default takes it from that one
    let todo = StateRow { is_default: true, ..named(&db, 10, "Todo") };
    assert!(block_on(db.update_state(&scope, &todo)).unwrap());
    assert_eq!(defaults(&db, 10), ["Todo"]);
    // a state that is the default and is changed to be none leaves the project with none
    let off = StateRow { is_default: false, ..named(&db, 10, "Todo") };
    assert!(block_on(db.update_state(&scope, &off)).unwrap());
    assert!(defaults(&db, 10).is_empty());
    // and one that is not there takes the default from no one
    assert!(block_on(db.update_state(&scope, &StateRow { is_default: true, ..named(&db, 10, "Review") })).unwrap());
    let nowhere = StateRow { is_default: true, ..new_state(10, 99, "Nowhere", 0.0) };
    assert!(!block_on(db.update_state(&scope, &nowhere)).unwrap());
    assert_eq!(defaults(&db, 10), ["Review"]);
    // the other project's default is its own
    make(&db, &new_project(A, 11, 2, &[], 1));
    assert_eq!(defaults(&db, 11), ["Backlog"]);
}

#[test]
fn a_name_is_taken_by_a_live_state_of_the_project_and_by_no_other() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    make(&db, &new_project(A, 11, 2, &[], 1));
    let scope = pscope(A, 10, 2);
    let taken =
        |r: Result<StateRow, DbError>| matches!(r, Err(DbError::Conflict(c)) if c == "states_project_id_name_key");
    assert!(taken(block_on(db.insert_state(&scope, &new_state(10, 1, "Todo", 0.0)))));
    // exact: another case is another name
    block_on(db.insert_state(&scope, &new_state(10, 2, "todo", 0.0))).unwrap();
    // another project's state may have the name
    block_on(db.insert_state(&pscope(A, 11, 2), &new_state(11, 1, "Mine", 0.0))).unwrap();
    block_on(db.insert_state(&scope, &new_state(10, 3, "Mine", 0.0))).unwrap();
    // a deleted state's name is free
    let todo = named(&db, 10, "Todo");
    assert!(block_on(db.delete_state(&scope, todo.id, at(11))).unwrap());
    block_on(db.insert_state(&scope, &new_state(10, 4, "Todo", 0.0))).unwrap();
    // a change cannot take a live name, and can keep its own
    let clash = StateRow { name: "Cancelled".into(), ..named(&db, 10, "Done") };
    assert!(matches!(block_on(db.update_state(&scope, &clash)), Err(DbError::Conflict(_))));
    assert!(block_on(db.update_state(&scope, &named(&db, 10, "Done"))).unwrap());
    // and the same id twice is a primary key
    let again = new_state(10, 4, "Another", 0.0);
    assert!(matches!(block_on(db.insert_state(&scope, &again)), Err(DbError::Conflict(c)) if c == "states_pkey"));
}

#[test]
fn a_change_replaces_what_it_names_keeps_the_rest_and_finds_only_live_states_of_the_project() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    make(&db, &new_project(A, 11, 2, &[], 1));
    let scope = pscope(A, 10, 2);
    let before = named(&db, 10, "Todo");
    let changed = StateRow {
        name: "Renamed".into(),
        description: "d".into(),
        color: "#abcdef".into(),
        sequence: 123.5,
        group: StateGroup::Completed,
        is_default: false,
        external_source: Some("gh".into()),
        external_id: Some("42".into()),
        updated_by: Some(uid(3)),
        updated_at: at(20),
        // what a change cannot move: the row is found by its id, in the scope's project
        created_by: Some(uid(7)),
        created_at: at(99),
        ..before.clone()
    };
    assert!(block_on(db.update_state(&scope, &changed)).unwrap());
    let after = named(&db, 10, "Renamed");
    assert_eq!(
        (&after.description, &after.color, after.sequence, after.group, &after.external_source, &after.external_id),
        (
            &"d".to_string(),
            &"#abcdef".to_string(),
            123.5,
            StateGroup::Completed,
            &Some("gh".to_string()),
            &Some("42".to_string())
        )
    );
    assert_eq!((after.updated_by, after.updated_at), (Some(uid(3)), at(20)));
    assert_eq!(
        (after.id, after.project_id, after.created_by, after.created_at),
        (before.id, before.project_id, before.created_by, before.created_at)
    );

    // a user who does not exist cannot be the editor, and nothing changes
    let ghost = StateRow { updated_by: Some(uid(99)), ..after.clone() };
    assert!(matches!(block_on(db.update_state(&scope, &ghost)), Err(DbError::Backend(_))));
    // a state of another project, one that is deleted and one that is nowhere are not there
    assert!(!block_on(db.update_state(&scope, &named(&db, 11, "Todo"))).unwrap());
    assert!(block_on(db.delete_state(&scope, after.id, at(21))).unwrap());
    assert!(!block_on(db.update_state(&scope, &after)).unwrap());
    assert!(!block_on(db.update_state(&scope, &new_state(10, 99, "Nowhere", 0.0))).unwrap());
}

#[test]
fn a_deleted_state_leaves_the_lists_and_is_still_read_by_its_work_items() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    make(&db, &new_project(A, 11, 2, &[], 1));
    let scope = pscope(A, 10, 2);
    let in_progress = named(&db, 10, "In Progress");
    let item = work_item_in(&db, 1, in_progress.id, false, false, false);
    let group = |db: &MemoryDatabase| block_on(db.work_item_row(&scope, item.id)).unwrap().unwrap().state_group;
    assert_eq!(group(&db), Some(StateGroup::Started));

    assert!(block_on(db.delete_state(&scope, in_progress.id, at(5))).unwrap());
    assert!(states(&db, 10).iter().all(|s| s.id != in_progress.id));
    let member = wscope(A, 1, Role::Admin);
    assert!(block_on(db.workspace_states(&member)).unwrap().iter().all(|s| s.id != in_progress.id));
    // the work item still names it, and reads its group
    assert_eq!(group(&db), Some(StateGroup::Started));
    assert_eq!(block_on(db.work_item_row(&scope, item.id)).unwrap().unwrap().attrs.state_id, Some(in_progress.id));

    // the intake state is deletable too, and `triage_state` then finds none
    let triage = block_on(db.triage_state(&scope)).unwrap().unwrap();
    assert!(block_on(db.delete_state(&scope, triage.id, at(6))).unwrap());
    assert!(block_on(db.triage_state(&scope)).unwrap().is_none());

    // a second deletion, another project's state and a state that is nowhere are not there
    assert!(!block_on(db.delete_state(&scope, in_progress.id, at(7))).unwrap());
    assert!(!block_on(db.delete_state(&scope, named(&db, 11, "Todo").id, at(7))).unwrap());
    assert!(!block_on(db.delete_state(&scope, state_id(10, 99), at(7))).unwrap());
}

/// The pin's `Issue.objects`: a work item counts unless it is a draft, archived or deleted, and none of a project that is archived
/// counts. Every kind of item in a state of its own, and the project archived last.
#[test]
fn work_items_in_a_state_count_as_the_pin_counts_them() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    let scope = pscope(A, 10, 2);
    let ids: Vec<Uuid> = states(&db, 10).iter().map(|s| s.id).collect();
    // (the work item, its state, a draft, archived, deleted, whether it counts)
    let kinds = [
        (1u128, ids[0], false, false, false, true),
        (2, ids[1], true, false, false, false),
        (3, ids[2], false, true, false, false),
        (4, ids[3], false, false, true, false),
    ];
    for (n, state, draft, archived, deleted, _) in kinds {
        work_item_in(&db, n, state, draft, archived, deleted);
    }
    for (n, state, draft, archived, deleted, counts) in kinds {
        let got = block_on(db.state_has_work_items(&scope, state)).unwrap();
        assert_eq!(got, counts, "item {n} (draft {draft}, archived {archived}, deleted {deleted})");
    }
    // a state with nothing in it, and a state that is nowhere
    assert!(!block_on(db.state_has_work_items(&scope, ids[4])).unwrap());
    assert!(!block_on(db.state_has_work_items(&scope, state_id(10, 99))).unwrap());
    // another project's scope does not see these states' items
    make(&db, &new_project(A, 11, 2, &[], 1));
    assert!(!block_on(db.state_has_work_items(&pscope(A, 11, 2), ids[0])).unwrap());
    // and once the project is archived none of its work items counts
    let admin = wscope(A, 1, Role::Admin);
    let mut row = block_on(db.project_detail(&admin, pid(10))).unwrap().unwrap().listing.project;
    row.archived_at = Some(at(30));
    assert!(block_on(db.update_project(&admin, &row)).unwrap());
    assert!(!block_on(db.state_has_work_items(&scope, ids[0])).unwrap());
}

#[test]
fn a_project_is_archived_when_it_says_so() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    let admin = wscope(A, 1, Role::Admin);
    assert!(!block_on(db.project_is_archived(&pscope(A, 10, 2))).unwrap());
    let mut row = block_on(db.project_detail(&admin, pid(10))).unwrap().unwrap().listing.project;
    row.archived_at = Some(at(5));
    assert!(block_on(db.update_project(&admin, &row)).unwrap());
    assert!(block_on(db.project_is_archived(&pscope(A, 10, 2))).unwrap());
    // a project that is not there, or is in another workspace, is not archived
    assert!(!block_on(db.project_is_archived(&pscope(A, 99, 2))).unwrap());
    assert!(!block_on(db.project_is_archived(&pscope(B, 10, 2))).unwrap());
    row.archived_at = None;
    assert!(block_on(db.update_project(&admin, &row)).unwrap());
    assert!(!block_on(db.project_is_archived(&pscope(A, 10, 2))).unwrap());
}

// --- labels (migration 0015) -------------------------------------------------------------------------------------------------

fn label_id(project: u128, k: u128) -> Uuid {
    Uuid::from_u128((project << 48) | (0x200 + k))
}

/// A label of project `project`, made by user 2, with the place `sort_order` (which `insert_label` replaces unless it is the first).
fn new_label(project: u128, k: u128, name: &str, sort_order: f64) -> LabelRow {
    LabelRow {
        id: label_id(project, k),
        workspace_id: wid(A),
        project_id: pid(project),
        parent_id: None,
        name: name.into(),
        description: String::new(),
        color: "#abcdef".into(),
        sort_order,
        created_by: Some(uid(2)),
        created_at: at(10),
        updated_at: at(10),
        external_source: None,
        external_id: None,
        updated_by: None,
    }
}

fn labels(db: &MemoryDatabase, project: u128) -> Vec<LabelRow> {
    block_on(db.project_labels(&pscope(A, project, 2))).unwrap()
}

fn label_names(db: &MemoryDatabase, project: u128) -> Vec<String> {
    labels(db, project).into_iter().map(|l| l.name).collect()
}

fn insert_label(db: &MemoryDatabase, project: u128, row: &LabelRow) -> LabelRow {
    block_on(db.insert_label(&pscope(A, project, 2), row)).unwrap()
}

/// A work item `n` of project 10 that carries `labels`, written with its attributes in one transaction.
fn work_item_with(db: &MemoryDatabase, n: u128, labels: &[Uuid]) -> WorkItem {
    let item = WorkItem {
        id: WorkItemId(Uuid::from_u128(n)),
        workspace_id: wid(A),
        project_id: pid(10),
        sequence_id: n as i64,
        sort_order: 1,
        name: format!("w{n}"),
        description: None,
        external_source: None,
        external_id: None,
        created_by: Some(uid(2)),
        created_at: at(0),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    let scope = pscope(A, 10, 2);
    let mut tx = block_on(db.begin(&scope)).unwrap();
    block_on(tx.insert_work_item(&scope, &item)).unwrap();
    let attrs = WorkItemAttrs { label_ids: labels.to_vec(), ..WorkItemAttrs::bare(at(0)) };
    block_on(tx.insert_work_item_attrs(&scope, item.id, &attrs)).unwrap();
    block_on(tx.commit()).unwrap();
    item
}

fn labels_of_item(db: &MemoryDatabase, item: &WorkItem) -> Vec<Uuid> {
    block_on(db.work_item_row(&pscope(A, 10, 2), item.id)).unwrap().unwrap().attrs.label_ids
}

#[test]
fn a_new_label_goes_after_the_greatest_live_one_by_ten_thousand() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[3], 0));
    let scope = pscope(A, 10, 2);
    // the first label of a project keeps the place it asked for
    let first = insert_label(&db, 10, &new_label(10, 1, "bug", 65535.0));
    assert_eq!(first.sort_order, 65535.0);
    // what is returned is what is stored
    assert_eq!(labels(&db, 10), vec![first.clone()]);
    // the next goes after it, whatever it asked for
    assert_eq!(insert_label(&db, 10, &new_label(10, 2, "feature", 1.0)).sort_order, 75535.0);
    let last = insert_label(&db, 10, &new_label(10, 3, "docs", 9e9));
    assert_eq!(last.sort_order, 85535.0);
    // a deleted label is not counted: the next goes where the deleted one went
    assert!(block_on(db.delete_label(&scope, last.id, at(11))).unwrap());
    assert_eq!(insert_label(&db, 10, &new_label(10, 4, "ops", 0.0)).sort_order, 85535.0);
    // another project's labels do not count: its own first keeps its place, and its second goes after that
    make(&db, &new_project(A, 11, 2, &[], 1));
    assert_eq!(insert_label(&db, 11, &new_label(11, 1, "bug", 7.0)).sort_order, 7.0);
    assert_eq!(insert_label(&db, 11, &new_label(11, 2, "feature", 0.0)).sort_order, 10007.0);
}

#[test]
fn a_label_name_is_taken_by_a_live_label_of_the_project_and_by_no_other() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    make(&db, &new_project(A, 11, 2, &[], 1));
    let scope = pscope(A, 10, 2);
    let bug = insert_label(&db, 10, &new_label(10, 1, "bug", 1.0));
    // the same name is refused, whatever else the row says
    let clash = block_on(db.insert_label(&scope, &new_label(10, 2, "bug", 1.0)));
    assert!(matches!(clash, Err(DbError::Conflict(_))), "{clash:?}");
    assert_eq!(label_names(&db, 10), ["bug"], "nothing was written");
    // the name is exact (`Bug` is another name), and another project may have it
    insert_label(&db, 10, &new_label(10, 3, "Bug", 1.0));
    insert_label(&db, 11, &new_label(11, 1, "bug", 1.0));
    // a deleted label's name is free again, and the new label is another row
    assert!(block_on(db.delete_label(&scope, bug.id, at(11))).unwrap());
    insert_label(&db, 10, &new_label(10, 4, "bug", 1.0));
    assert_eq!(label_names(&db, 10), ["Bug", "bug"]);
    // an id that is taken, a label of another project and a user who is not there are refused
    assert!(matches!(block_on(db.insert_label(&scope, &new_label(10, 4, "other", 1.0))), Err(DbError::Conflict(_))));
    assert!(matches!(block_on(db.insert_label(&scope, &new_label(11, 9, "x", 1.0))), Err(DbError::Backend(_))));
    let ghost = LabelRow { created_by: Some(uid(99)), ..new_label(10, 5, "ghost", 1.0) };
    assert!(matches!(block_on(db.insert_label(&scope, &ghost)), Err(DbError::Backend(_))));
    let orphan = LabelRow { parent_id: Some(label_id(10, 99)), ..new_label(10, 6, "orphan", 1.0) };
    assert!(matches!(block_on(db.insert_label(&scope, &orphan)), Err(DbError::Backend(_))));
}

#[test]
fn a_label_change_replaces_what_it_names_keeps_the_rest_and_finds_only_live_labels_of_the_project() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    make(&db, &new_project(A, 11, 2, &[], 1));
    let scope = pscope(A, 10, 2);
    let a = insert_label(&db, 10, &new_label(10, 1, "bug", 1.0));
    let b = insert_label(&db, 10, &new_label(10, 2, "feature", 1.0));
    let mut asked = LabelRow {
        parent_id: Some(b.id),
        name: "defect".into(),
        description: "d".into(),
        color: "#000".into(),
        sort_order: 1.5,
        external_source: Some("gh".into()),
        external_id: Some("7".into()),
        updated_by: Some(uid(3)),
        updated_at: at(20),
        ..a.clone()
    };
    // who made the label, and when, are not for a change to rewrite
    asked.created_by = Some(uid(4));
    asked.created_at = at(99);
    assert!(block_on(db.update_label(&scope, &asked)).unwrap());
    let stored = labels(&db, 10).into_iter().find(|l| l.id == a.id).unwrap();
    assert_eq!(stored, LabelRow { created_by: a.created_by, created_at: a.created_at, ..asked.clone() });
    // a name that another live label has is a conflict and changes nothing; the label's own name is not
    let onto = LabelRow { name: "defect".into(), ..b.clone() };
    assert!(matches!(block_on(db.update_label(&scope, &onto)), Err(DbError::Conflict(_))));
    assert_eq!(labels(&db, 10).into_iter().find(|l| l.id == b.id).unwrap(), b);
    assert!(block_on(db.update_label(&scope, &LabelRow { color: "#111".into(), ..b.clone() })).unwrap());
    // a parent that is no label, and an editor who is no user, are refused and change nothing
    let orphan = LabelRow { parent_id: Some(label_id(10, 99)), ..b.clone() };
    assert!(matches!(block_on(db.update_label(&scope, &orphan)), Err(DbError::Backend(_))));
    let ghost = LabelRow { updated_by: Some(uid(99)), ..b.clone() };
    assert!(matches!(block_on(db.update_label(&scope, &ghost)), Err(DbError::Backend(_))));
    assert_eq!(labels(&db, 10).into_iter().find(|l| l.id == b.id).unwrap().color, "#111");
    // a label of another project, one that is nowhere and a deleted one are not there to change
    let other = insert_label(&db, 11, &new_label(11, 1, "bug", 1.0));
    assert!(!block_on(db.update_label(&scope, &LabelRow { project_id: pid(10), ..other.clone() })).unwrap());
    assert!(!block_on(db.update_label(&scope, &LabelRow { id: label_id(10, 98), ..b.clone() })).unwrap());
    assert!(block_on(db.delete_label(&scope, b.id, at(30))).unwrap());
    assert!(!block_on(db.update_label(&scope, &b)).unwrap());
    assert_eq!(labels(&db, 11), vec![other], "another project's label is untouched");
}

#[test]
fn a_deleted_label_takes_the_labels_under_it_and_the_hold_of_every_work_item_on_all_of_them() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    make(&db, &new_project(A, 11, 2, &[], 1));
    let scope = pscope(A, 10, 2);
    let parent = insert_label(&db, 10, &new_label(10, 1, "parent", 1.0));
    let child = insert_label(&db, 10, &LabelRow { parent_id: Some(parent.id), ..new_label(10, 2, "child", 1.0) });
    let grandchild =
        insert_label(&db, 10, &LabelRow { parent_id: Some(child.id), ..new_label(10, 3, "grandchild", 1.0) });
    let sibling = insert_label(&db, 10, &new_label(10, 4, "sibling", 1.0));
    let other = insert_label(&db, 11, &new_label(11, 1, "parent", 1.0));
    let both = work_item_with(&db, 1, &[parent.id, sibling.id]);
    let deep = work_item_with(&db, 2, &[grandchild.id, sibling.id]);
    let kept = work_item_with(&db, 3, &[sibling.id]);
    let none = work_item_with(&db, 4, &[]);

    // the child alone: it and what is under it go, the parent and the sibling stay
    assert!(block_on(db.delete_label(&scope, child.id, at(20))).unwrap());
    assert_eq!(label_names(&db, 10), ["parent", "sibling"]);
    assert_eq!(labels_of_item(&db, &deep), [sibling.id]);
    assert_eq!(labels_of_item(&db, &both), [parent.id, sibling.id]);

    // then the parent, which has nothing under it now
    assert!(block_on(db.delete_label(&scope, parent.id, at(21))).unwrap());
    assert_eq!(label_names(&db, 10), ["sibling"]);
    assert_eq!(labels_of_item(&db, &both), [sibling.id]);
    assert_eq!(labels_of_item(&db, &kept), [sibling.id]);
    assert!(labels_of_item(&db, &none).is_empty());
    // a label of another project that has the name is not touched
    assert_eq!(labels(&db, 11), vec![other.clone()]);

    // a second deletion, another project's label and a label that is nowhere are not there
    assert!(!block_on(db.delete_label(&scope, parent.id, at(22))).unwrap());
    assert!(!block_on(db.delete_label(&scope, other.id, at(22))).unwrap());
    assert!(!block_on(db.delete_label(&scope, label_id(10, 99), at(22))).unwrap());
    // the names of the deleted are free
    insert_label(&db, 10, &new_label(10, 5, "parent", 1.0));
    insert_label(&db, 10, &new_label(10, 6, "child", 1.0));
}

/// The pin lets a label be its own parent, and two labels each other's: the walk down a deleted label must end, and take the whole loop.
#[test]
fn a_label_that_is_its_own_parent_or_in_a_loop_is_deleted_with_what_is_in_it() {
    let db = world();
    make(&db, &new_project(A, 10, 2, &[], 0));
    let scope = pscope(A, 10, 2);
    let a = insert_label(&db, 10, &new_label(10, 1, "a", 1.0));
    let b = insert_label(&db, 10, &new_label(10, 2, "b", 1.0));
    let c = insert_label(&db, 10, &new_label(10, 3, "c", 1.0));
    let alone = insert_label(&db, 10, &new_label(10, 4, "alone", 1.0));
    // a is its own parent; b and c are each other's
    assert!(block_on(db.update_label(&scope, &LabelRow { parent_id: Some(a.id), ..a.clone() })).unwrap());
    assert!(block_on(db.update_label(&scope, &LabelRow { parent_id: Some(c.id), ..b.clone() })).unwrap());
    assert!(block_on(db.update_label(&scope, &LabelRow { parent_id: Some(b.id), ..c.clone() })).unwrap());
    let item = work_item_with(&db, 1, &[a.id, b.id, c.id, alone.id]);
    assert!(block_on(db.delete_label(&scope, a.id, at(20))).unwrap());
    assert_eq!(label_names(&db, 10), ["b", "c", "alone"]);
    assert!(block_on(db.delete_label(&scope, c.id, at(21))).unwrap());
    assert_eq!(label_names(&db, 10), ["alone"], "the loop goes whole, from either end");
    assert_eq!(labels_of_item(&db, &item), [alone.id]);
}

proptest! {
    #![proptest_config(proptest_config(128))]

    /// In any forest of labels (a parent may be any label, itself and a loop included), deleting one deletes exactly the labels that
    /// reach it by parents, and every work item that held labels keeps exactly the survivors. The model is the closure, computed here.
    #[test]
    fn prop_deleting_a_label_deletes_exactly_what_reaches_it_by_parents(
        (parents, victim) in (1usize..9).prop_flat_map(|n| (prop::collection::vec(prop::option::of(0..n), n), 0..n)),
    ) {
        let db = world();
        make(&db, &new_project(A, 10, 2, &[], 0));
        let scope = pscope(A, 10, 2);
        let rows: Vec<LabelRow> =
            (0..parents.len()).map(|i| insert_label(&db, 10, &new_label(10, i as u128, &format!("l{i}"), 1.0))).collect();
        for (row, parent) in rows.iter().zip(&parents) {
            let parent_id = parent.map(|p| rows[p].id);
            let changed = block_on(db.update_label(&scope, &LabelRow { parent_id, ..row.clone() })).unwrap();
            prop_assert!(changed);
        }
        let all: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        let item = work_item_with(&db, 1, &all);

        // the model: the victim, and every label whose chain of parents meets a label already in the set
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
        prop_assert!(block_on(db.delete_label(&scope, rows[victim].id, at(20))).unwrap());
        let mut want: Vec<Uuid> = (0..rows.len()).filter(|i| !gone.contains(i)).map(|i| rows[i].id).collect();
        want.sort();
        let mut live: Vec<Uuid> = labels(&db, 10).into_iter().map(|l| l.id).collect();
        live.sort();
        prop_assert_eq!(live, want.clone());
        let mut held = labels_of_item(&db, &item);
        held.sort();
        prop_assert_eq!(held, want);
    }
}
