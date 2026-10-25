//! What the app reads when a member opens a workspace, in the memory adapter (issue #44, migrations 0010 and 0011): the
//! projects the member can see, and the rows a first visit makes (the sidebar, the home page's widgets, the work-item list
//! settings). The Postgres adapter is held to the same answers by the generated sequences in
//! `thrust-adapters-selfhosted/tests/model`.
//!
//! The rules, written out:
//!
//! * `workspace_projects` lists the projects of the scope's workspace, newest first, ties by id, and no other workspace's. An
//!   admin of the workspace sees all of them, secret and archived ones included; a member sees the ones they are in and the
//!   public ones; a guest sees only the ones they are in. A project's member row counts as the caller's role only when it
//!   names the project's own workspace. The caller's own sort order for a project is shown when they have a row for it.
//! * `ensure_sidebar_preferences`, `ensure_home_preferences` and `ensure_user_properties` make the caller's rows that are not
//!   there yet and return what is there. A row that exists is never changed by asking again, whatever the new defaults are;
//!   one member's rows, or one workspace's, are not another's. The sidebar comes back by `sort_order` (ties by key), the
//!   home page's widgets by key.

mod common;

use std::collections::BTreeMap;

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

const CALLER: UserId = UserId(Uuid::from_u128(1));
const HERE: WorkspaceId = WorkspaceId(Uuid::from_u128(0xA));
const THERE: WorkspaceId = WorkspaceId(Uuid::from_u128(0xB));

fn scope(ws: WorkspaceId, user: UserId, role: Role) -> WorkspaceScope {
    WorkspaceScope::mint(ws, user, role)
}

/// A project of the tests' world: where it is, what it is, and how the caller is tied to it.
#[derive(Debug, Clone)]
struct Proj {
    id: u128,
    ws: WorkspaceId,
    network: ProjectNetwork,
    archived: bool,
    /// Seconds after the tests' epoch; equal values make a tie that the id breaks.
    created: i64,
    /// The caller's role in it, and the workspace the member row names (the project's own, or the other one).
    member: Option<(Role, WorkspaceId)>,
    sort_order: Option<f64>,
}

fn world(projects: &[Proj]) -> MemoryDatabase {
    let db = MemoryDatabase::new();
    let f = SeedFixture {
        workspaces: vec![
            SeedWorkspace { id: HERE, slug: WorkspaceSlug::new("here") },
            SeedWorkspace { id: THERE, slug: WorkspaceSlug::new("there") },
        ],
        users: vec![CALLER],
        workspace_members: vec![
            SeedWorkspaceMember { workspace_id: HERE, user_id: CALLER },
            SeedWorkspaceMember { workspace_id: THERE, user_id: CALLER },
        ],
        projects: projects.iter().map(|p| SeedProject { id: pid(p.id), workspace_id: p.ws, name: format!("p{}", p.id) }).collect(),
        project_details: projects
            .iter()
            .map(|p| SeedProjectDetails {
                network: p.network,
                archived: p.archived,
                created_at: Some(at(p.created)),
                ..SeedProjectDetails::of(pid(p.id))
            })
            .collect(),
        memberships: projects
            .iter()
            .filter_map(|p| p.member.map(|(role, ws)| SeedMembership { project_id: pid(p.id), workspace_id: ws, user_id: CALLER, role }))
            .collect(),
        project_sort_orders: projects
            .iter()
            .filter_map(|p| p.sort_order.map(|sort_order| SeedProjectSortOrder { project_id: pid(p.id), user_id: CALLER, sort_order }))
            .collect(),
        ..SeedFixture::default()
    };
    block_on(db.seed(&sys(), &f)).unwrap();
    db
}

fn listed(db: &MemoryDatabase, ws: WorkspaceId, role: Role) -> Vec<ProjectListing> {
    block_on(db.workspace_projects(&scope(ws, CALLER, role))).unwrap()
}

// --- the project list ----------------------------------------------------------------------------------------------------

/// The spec's table: whether a caller of this workspace role sees a project of this network, given their own role in it.
fn sees(role: Role, network: ProjectNetwork, theirs: Option<Role>) -> bool {
    match role {
        Role::Admin => true,
        Role::Member => theirs.is_some() || network == ProjectNetwork::Public,
        Role::Guest => theirs.is_some(),
    }
}

#[rstest]
fn a_project_is_seen_as_the_workspace_role_and_the_network_allow(
    #[values(Role::Guest, Role::Member, Role::Admin)] role: Role,
    #[values(ProjectNetwork::Secret, ProjectNetwork::Public)] network: ProjectNetwork,
    #[values(None, Some(Role::Guest), Some(Role::Member), Some(Role::Admin))] theirs: Option<Role>,
    #[values(false, true)] archived: bool,
) {
    let p = Proj { id: 1, ws: HERE, network, archived, created: 0, member: theirs.map(|r| (r, HERE)), sort_order: None };
    let got = listed(&world(&[p]), HERE, role);
    assert_eq!(!got.is_empty(), sees(role, network, theirs), "{role:?} {network:?} {theirs:?} archived={archived}");
    for l in &got {
        // what is listed says the caller's own role in it, and whether it is archived, whoever the caller is
        assert_eq!(l.role, theirs);
        assert_eq!(l.project.archived_at.is_some(), archived);
        assert_eq!(l.project.network, network);
    }
}

/// A member row is the caller's role in a project only when it names the project's own workspace: one written with another
/// workspace is no role (the join on `m.workspace_id` in Postgres), so the project is seen as one the caller is not in.
#[rstest]
fn a_member_row_of_another_workspace_gives_no_role_and_no_access(
    #[values(Role::Guest, Role::Member, Role::Admin)] role: Role,
    #[values(ProjectNetwork::Secret, ProjectNetwork::Public)] network: ProjectNetwork,
) {
    let p = Proj { id: 1, ws: HERE, network, archived: false, created: 0, member: Some((Role::Admin, THERE)), sort_order: None };
    let got = listed(&world(&[p]), HERE, role);
    assert_eq!(!got.is_empty(), sees(role, network, None), "{role:?} {network:?}");
    assert!(got.iter().all(|l| l.role.is_none()), "{got:?}");
}

#[test]
fn the_callers_sort_order_is_shown_only_where_they_have_one() {
    let with = Proj { id: 1, ws: HERE, network: ProjectNetwork::Public, archived: false, created: 0, member: None, sort_order: Some(65535.0) };
    let without = Proj { id: 2, sort_order: None, ..with.clone() };
    let got = listed(&world(&[with, without]), HERE, Role::Admin);
    let order = |id: u128| got.iter().find(|l| l.project.id == pid(id)).unwrap().sort_order;
    assert_eq!((order(1), order(2)), (Some(65535.0), None));
}

fn any_proj() -> impl Strategy<Value = Proj> {
    (
        prop_oneof![Just(HERE), Just(THERE)],
        prop_oneof![Just(ProjectNetwork::Secret), Just(ProjectNetwork::Public)],
        any::<bool>(),
        // few values, so that creation times tie
        0..4i64,
        prop::option::of((prop_oneof![Just(Role::Guest), Just(Role::Member), Just(Role::Admin)], prop_oneof![Just(HERE), Just(THERE)])),
        prop::option::of(prop_oneof![Just(0.0), Just(65535.0), Just(-1.5), Just(1e9)]),
    )
        .prop_map(|(ws, network, archived, created, member, sort_order)| Proj { id: 0, ws, network, archived, created, member, sort_order })
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// Whatever the projects are, a caller of any role gets exactly the projects of the scope's workspace that the table above
    /// lets them see, newest first with ties by id, each with its own facts. Project ids are given out in a scrambled order so
    /// that the id tie-break is not the order they were made in.
    #[test]
    fn prop_the_list_is_the_visible_projects_of_this_workspace_newest_first(
        mut projects in prop::collection::vec(any_proj(), 0..12),
        role in prop_oneof![Just(Role::Guest), Just(Role::Member), Just(Role::Admin)],
        ws in prop_oneof![Just(HERE), Just(THERE)],
    ) {
        for (i, p) in projects.iter_mut().enumerate() {
            p.id = 0x100 + (i as u128 * 7) % 13;
        }
        // a project id the world is built from is unique
        projects.sort_by_key(|p| p.id);
        projects.dedup_by_key(|p| p.id);
        let got = listed(&world(&projects), ws, role);

        let mut want: Vec<&Proj> = projects
            .iter()
            .filter(|p| p.ws == ws)
            .filter(|p| sees(role, p.network, p.member.filter(|(_, row_ws)| *row_ws == p.ws).map(|(r, _)| r)))
            .collect();
        want.sort_by(|a, b| b.created.cmp(&a.created).then(b.id.cmp(&a.id)));

        let got_ids: Vec<ProjectId> = got.iter().map(|l| l.project.id).collect();
        let want_ids: Vec<ProjectId> = want.iter().map(|p| pid(p.id)).collect();
        prop_assert_eq!(got_ids, want_ids);
        for (l, p) in got.iter().zip(&want) {
            prop_assert_eq!(l.role, p.member.filter(|(_, row_ws)| *row_ws == p.ws).map(|(r, _)| r));
            prop_assert_eq!(l.sort_order, p.sort_order);
            prop_assert_eq!(l.project.workspace_id, p.ws);
            prop_assert_eq!(l.project.created_at, at(p.created));
            prop_assert_eq!(&l.project.identifier, &default_project_identifier(pid(p.id)));
        }
    }
}

proptest! {
    #![proptest_config(proptest_config(128))]

    /// A seeded membership has an id of its own, made from the workspace and the user: two members never share one, whatever
    /// workspaces and users they are of (Postgres keeps a unique index on it, and the app names a member by it).
    #[test]
    fn prop_seeded_members_have_ids_of_their_own(
        pairs in prop::collection::btree_set((1..5u128, 1..6u128), 1..12),
    ) {
        let db = MemoryDatabase::new();
        // workspace ids that use the high half of the id as well as the low, as random ones do, and users that use the low half
        let big = |w: u128| wid((w << 64) | w);
        let workspaces: std::collections::BTreeSet<u128> = pairs.iter().map(|p| p.0).collect();
        let users: std::collections::BTreeSet<u128> = pairs.iter().map(|p| p.1).collect();
        let f = SeedFixture {
            workspaces: workspaces.iter().map(|w| SeedWorkspace { id: big(*w), slug: WorkspaceSlug::new(format!("ws-{w}")) }).collect(),
            users: users.iter().map(|u| uid(*u)).collect(),
            workspace_members: pairs.iter().map(|(w, u)| SeedWorkspaceMember { workspace_id: big(*w), user_id: uid(*u) }).collect(),
            ..SeedFixture::default()
        };
        block_on(db.seed(&sys(), &f)).unwrap();
        let mut ids = std::collections::BTreeSet::new();
        for (w, u) in &pairs {
            // any member of the workspace may list its members
            let rows = block_on(db.workspace_members(&scope(big(*w), uid(*u), Role::Member))).unwrap();
            for r in rows.iter().filter(|r| r.member.workspace_id == big(*w)) {
                ids.insert((r.member.workspace_id, r.member.user_id, r.member.id));
            }
        }
        let distinct: std::collections::BTreeSet<Uuid> = ids.iter().map(|(_, _, id)| *id).collect();
        prop_assert_eq!(ids.len(), pairs.len());
        prop_assert_eq!(distinct.len(), pairs.len(), "two seeded members share an id");
    }
}

// --- a first visit ---------------------------------------------------------------------------------------------------------

/// One call a member's first or later visit makes. `salt` makes the defaults of one call differ from another's, so a row
/// that was changed by asking again shows.
#[derive(Debug, Clone)]
enum Visit {
    Sidebar { mask: u8, salt: u8 },
    Home { mask: u8, salt: u8 },
    Properties { salt: u8 },
}

const KEYS: [&str; 5] = ["alpha", "beta", "gamma", "delta", "epsilon"];

fn visit() -> impl Strategy<Value = (Visit, usize, usize)> {
    (
        prop_oneof![
            (1..32u8, 0..4u8).prop_map(|(mask, salt)| Visit::Sidebar { mask, salt }),
            (1..32u8, 0..4u8).prop_map(|(mask, salt)| Visit::Home { mask, salt }),
            (0..4u8).prop_map(|salt| Visit::Properties { salt }),
        ],
        // which workspace and which member: two of each
        0..2usize,
        0..2usize,
    )
}

fn sidebar_defaults(mask: u8, salt: u8) -> Vec<SidebarPreference> {
    (0..5)
        .filter(|i| mask & (1 << i) != 0)
        // sort orders collide on purpose (two keys at one order), and are fractional
        .map(|i| SidebarPreference { key: KEYS[i].to_string(), is_pinned: (i as u8 + salt) % 2 == 0, sort_order: f64::from((i as u8 + salt) % 3) + 0.5 })
        .collect()
}

fn home_defaults(mask: u8, salt: u8) -> Vec<HomePreference> {
    (0..5)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| HomePreference {
            key: KEYS[i].to_string(),
            is_enabled: (i as u8 + salt) % 2 == 1,
            config: json!({"salt": salt, "i": i}),
            sort_order: 100.0 - i as f64 - f64::from(salt),
            created_at: at(i64::from(salt)),
        })
        .collect()
}

fn properties(salt: u8) -> UserProperties {
    UserProperties {
        id: Uuid::from_u128(0x5000 + u128::from(salt)),
        filters: json!({"salt": salt}),
        display_filters: json!({"display_filters": {"salt": salt}}),
        display_properties: json!({"display_properties": {}}),
        rich_filters: json!({}),
        navigation_project_limit: 10 + i32::from(salt),
        navigation_control_preference: if salt % 2 == 0 { "ACCORDION" } else { "TABBED" }.to_string(),
        created_at: at(i64::from(salt)),
        updated_at: at(i64::from(salt)),
    }
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// Against the plain rule: for each (workspace, member) the first row that was ever asked for a key stays, and nothing else
    /// changes it. Every answer is everything the pair has so far, in the order the port names.
    #[test]
    fn prop_a_visit_makes_the_rows_that_are_missing_and_leaves_the_others_as_they_are(
        visits in prop::collection::vec(visit(), 1..24),
    ) {
        let db = MemoryDatabase::new();
        let (workspaces, users) = ([HERE, THERE], [CALLER, uid(2)]);
        let mut sidebar: BTreeMap<(usize, usize, String), SidebarPreference> = BTreeMap::new();
        let mut home: BTreeMap<(usize, usize, String), HomePreference> = BTreeMap::new();
        let mut props: BTreeMap<(usize, usize), UserProperties> = BTreeMap::new();
        for (v, w, u) in visits {
            let s = scope(workspaces[w], users[u], Role::Member);
            match v {
                Visit::Sidebar { mask, salt } => {
                    let defaults = sidebar_defaults(mask, salt);
                    for d in &defaults {
                        sidebar.entry((w, u, d.key.clone())).or_insert_with(|| d.clone());
                    }
                    let mut want: Vec<SidebarPreference> =
                        sidebar.iter().filter(|((sw, su, _), _)| (*sw, *su) == (w, u)).map(|(_, p)| p.clone()).collect();
                    want.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order).then_with(|| a.key.cmp(&b.key)));
                    prop_assert_eq!(block_on(db.ensure_sidebar_preferences(&s, &defaults, at(0))).unwrap(), want);
                }
                Visit::Home { mask, salt } => {
                    let defaults = home_defaults(mask, salt);
                    for d in &defaults {
                        home.entry((w, u, d.key.clone())).or_insert_with(|| d.clone());
                    }
                    // the map is ordered by key
                    let want: Vec<HomePreference> =
                        home.iter().filter(|((sw, su, _), _)| (*sw, *su) == (w, u)).map(|(_, p)| p.clone()).collect();
                    prop_assert_eq!(block_on(db.ensure_home_preferences(&s, &defaults)).unwrap(), want);
                }
                Visit::Properties { salt } => {
                    let new = properties(salt);
                    let want = props.entry((w, u)).or_insert_with(|| new.clone()).clone();
                    prop_assert_eq!(block_on(db.ensure_user_properties(&s, &new)).unwrap(), want);
                }
            }
        }
    }
}

/// The three kinds of row are separate: the sidebar's keys are not the home page's, though both are made by one member for one
/// workspace, and asking for one makes none of the other.
#[test]
fn the_sidebar_the_home_page_and_the_list_settings_are_separate_rows() {
    let db = MemoryDatabase::new();
    let s = scope(HERE, CALLER, Role::Member);
    block_on(db.ensure_sidebar_preferences(&s, &sidebar_defaults(0b11111, 0), at(0))).unwrap();
    assert_eq!(block_on(db.ensure_home_preferences(&s, &[])).unwrap(), vec![]);
    assert_eq!(block_on(db.ensure_sidebar_preferences(&s, &[], at(0))).unwrap().len(), 5);
}
