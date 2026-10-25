//! The workspace tables in the memory adapter (issue #44, migration 0009): a workspace and its members, who sees what, and
//! the roles in a workspace's projects. Exhaustive over the finite domains (a member is active, deactivated or absent; a
//! creation clashes on its id, its slug, its member's id or its owner, or does not) with the orderings spelled out, and
//! a property for the project roles, whose inputs are a set of rows. The Postgres adapter is held to the same answers by the
//! generated sequences in `thrust-adapters-selfhosted/tests/model`; what that model cannot reach (rows whose timestamps it
//! chooses, several projects) is here.

mod common;

use common::proptest_config;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
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

const OWNER: UserId = UserId(Uuid::from_u128(1));
const OTHER: UserId = UserId(Uuid::from_u128(2));

fn db_with_users(users: &[UserId]) -> MemoryDatabase {
    let db = MemoryDatabase::new();
    let rows: Vec<UserRecord> =
        users.iter().map(|u| UserRecord { id: *u, email: None, password_hash: None, is_active: true }).collect();
    block_on(db.seed_users(&sys(), &rows)).unwrap();
    db
}

/// A workspace `n` and its first member `owner` (member row id `0x1000 + n`), created `created` seconds after the epoch of
/// these tests, slug `ws-<n>`.
fn new_ws(n: u128, owner: UserId, role: Role, created: i64) -> NewWorkspace {
    NewWorkspace {
        workspace: Workspace {
            id: wid(n),
            slug: WorkspaceSlug::new(format!("ws-{n}")),
            name: format!("Workspace {n}"),
            owner_id: Some(owner),
            organization_size: Some("2-10".into()),
            timezone: "Europe/Berlin".into(),
            background_color: "#123456".into(),
            created_at: at(created),
            updated_at: at(created),
        },
        owner: WorkspaceMember {
            id: Uuid::from_u128(0x1000 + n),
            workspace_id: wid(n),
            user_id: owner,
            role,
            company_role: Some("lead".into()),
            is_active: true,
            props: serde_json::json!({"k": [1, 2]}),
            created_at: at(created),
            updated_at: at(created),
        },
    }
}

fn scope(ws: WorkspaceId, user: UserId) -> WorkspaceScope {
    WorkspaceScope::mint(ws, user, Role::Member)
}

fn seed_members(db: &MemoryDatabase, ws: WorkspaceId, users: &[UserId]) {
    let f = SeedFixture {
        users: users.to_vec(),
        workspace_members: users.iter().map(|u| SeedWorkspaceMember { workspace_id: ws, user_id: *u }).collect(),
        ..SeedFixture::default()
    };
    block_on(db.seed(&sys(), &f)).unwrap();
}

// --- making a workspace -----------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clash {
    Nothing,
    Id,
    Slug,
    MemberId,
    /// The owner is no user.
    Owner,
}

/// A creation either makes the workspace and its member or does nothing: the four ways it can fail leave no row behind, and
/// the workspace that was there is as it was. Only a clash on a unique value is a `Conflict`; an owner that is no user is a
/// `Backend` error, as a foreign key is in Postgres.
#[rstest]
fn a_creation_makes_a_workspace_and_its_member_together_or_not_at_all(
    #[values(Clash::Nothing, Clash::Id, Clash::Slug, Clash::MemberId, Clash::Owner)] clash: Clash,
) {
    let db = db_with_users(&[OWNER, OTHER]);
    let first = new_ws(10, OWNER, Role::Admin, 0);
    block_on(db.insert_workspace(&first)).unwrap();

    let mut second = new_ws(20, OTHER, Role::Member, 5);
    match clash {
        Clash::Nothing => {}
        Clash::Id => second.workspace.id = first.workspace.id,
        Clash::Slug => second.workspace.slug = first.workspace.slug.clone(),
        Clash::MemberId => second.owner.id = first.owner.id,
        Clash::Owner => second.workspace.owner_id = Some(uid(99)),
    }
    let got = block_on(db.insert_workspace(&second));
    match clash {
        Clash::Nothing => got.unwrap(),
        Clash::Owner => assert!(matches!(got, Err(DbError::Backend(_))), "{got:?}"),
        _ => assert!(matches!(got, Err(DbError::Conflict(_))), "{clash:?}: {got:?}"),
    }

    let theirs = block_on(db.workspaces_of(OTHER)).unwrap();
    if clash == Clash::Nothing {
        assert_eq!(theirs.len(), 1);
        assert_eq!(theirs[0].workspace, second.workspace);
    } else {
        assert!(theirs.is_empty(), "{clash:?}: the loser has a workspace: {theirs:?}");
        // nothing of the loser's: not its member row, not its slug
        assert_eq!(block_on(db.workspace_member(&scope(second.workspace.id, OTHER))).unwrap(), None);
        if clash != Clash::Slug {
            assert!(!block_on(db.workspace_slug_taken(second.workspace.slug.as_str())).unwrap(), "{clash:?}: the loser's slug is taken");
        }
    }
    // the winner is as it was
    let mine = block_on(db.workspaces_of(OWNER)).unwrap();
    assert_eq!(mine, vec![WorkspaceListing { workspace: first.workspace.clone(), role: Role::Admin, total_members: 1 }]);
    assert_eq!(block_on(db.workspace_member(&scope(first.workspace.id, OWNER))).unwrap(), Some(first.owner));
}

#[derive(Debug, Clone, Copy)]
enum How {
    Nothing,
    Seeded,
    Inserted,
}

#[rstest]
fn any_workspace_is_true_once_one_exists_however_it_was_made(#[values(How::Nothing, How::Seeded, How::Inserted)] how: How) {
    let db = db_with_users(&[OWNER]);
    match how {
        How::Nothing => {}
        How::Seeded => {
            let f = SeedFixture { workspaces: vec![SeedWorkspace { id: wid(1), slug: WorkspaceSlug::new("seeded") }], ..SeedFixture::default() };
            block_on(db.seed(&sys(), &f)).unwrap();
        }
        How::Inserted => block_on(db.insert_workspace(&new_ws(1, OWNER, Role::Admin, 0))).unwrap(),
    }
    assert_eq!(block_on(db.any_workspace()).unwrap(), !matches!(how, How::Nothing));
}

#[derive(Debug, Clone, Copy)]
enum Probe {
    Exact,
    Upper,
    Prefix,
    Padded,
    Other,
}

/// The slug check is exact: not case-insensitive, not a prefix, not trimmed.
#[rstest]
fn the_slug_check_is_exact(#[values(Probe::Exact, Probe::Upper, Probe::Prefix, Probe::Padded, Probe::Other)] probe: Probe) {
    let db = db_with_users(&[OWNER]);
    block_on(db.insert_workspace(&new_ws(7, OWNER, Role::Admin, 0))).unwrap();
    let slug = match probe {
        Probe::Exact => "ws-7".to_owned(),
        Probe::Upper => "WS-7".to_owned(),
        Probe::Prefix => "ws-".to_owned(),
        Probe::Padded => " ws-7".to_owned(),
        Probe::Other => "ws-8".to_owned(),
    };
    assert_eq!(block_on(db.workspace_slug_taken(&slug)).unwrap(), matches!(probe, Probe::Exact), "{slug:?}");
}

// --- who sees a workspace ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Active,
    Deactivated,
    NotAMember,
}

/// A deactivated member keeps the row, and loses the workspace everywhere else: the slug lookup, their list. The
/// membership row itself and the member list still show them.
#[rstest]
fn only_an_active_member_has_the_workspace_by_slug_and_in_their_list(
    #[values(State::Active, State::Deactivated, State::NotAMember)] state: State,
    #[values(true, false)] slug_known: bool,
) {
    let db = db_with_users(&[OWNER, OTHER]);
    block_on(db.insert_workspace(&new_ws(10, OWNER, Role::Admin, 0))).unwrap();
    if state != State::NotAMember {
        seed_members(&db, wid(10), &[OTHER]);
    }
    if state == State::Deactivated {
        assert!(block_on(db.set_workspace_member_active(&sys(), wid(10), OTHER, false)).unwrap());
    }
    let slug = WorkspaceSlug::new(if slug_known { "ws-10" } else { "ws-11" });
    let want_access = (slug_known && state == State::Active).then_some(WorkspaceAccess { workspace_id: wid(10), role: Role::Member });
    assert_eq!(block_on(db.workspace_access(&slug, OTHER)).unwrap(), want_access);

    let listed: Vec<WorkspaceId> = block_on(db.workspaces_of(OTHER)).unwrap().iter().map(|l| l.workspace.id).collect();
    assert_eq!(listed, if state == State::Active { vec![wid(10)] } else { vec![] });

    let row = block_on(db.workspace_member(&scope(wid(10), OTHER))).unwrap();
    assert_eq!(row.as_ref().map(|m| m.is_active), match state {
        State::Active => Some(true),
        State::Deactivated => Some(false),
        State::NotAMember => None,
    });
    let in_list = block_on(db.workspace_members(&scope(wid(10), OWNER))).unwrap().iter().any(|m| m.member.user_id == OTHER);
    assert_eq!(in_list, state != State::NotAMember, "the member list shows a deactivated member and not a stranger");
}

/// `set_workspace_member_active` answers whether the membership exists, for each way it might not.
#[rstest]
fn deactivating_answers_whether_the_membership_exists(
    #[values(10, 11)] workspace: u128,
    #[values(1, 2)] user: u128,
) {
    let db = db_with_users(&[OWNER, OTHER]);
    block_on(db.insert_workspace(&new_ws(10, OWNER, Role::Admin, 0))).unwrap();
    block_on(db.insert_workspace(&new_ws(11, OTHER, Role::Admin, 1))).unwrap();
    // OWNER belongs to workspace 10 only, OTHER to 11 only
    let exists = (workspace, user) == (10, 1) || (workspace, user) == (11, 2);
    assert_eq!(block_on(db.set_workspace_member_active(&sys(), wid(workspace), uid(user), false)).unwrap(), exists);
}

/// `total_members` counts the active members, whatever the others are, and tells the caller's own role.
#[rstest]
fn the_member_count_is_the_active_members(#[values(0, 1, 2, 3)] deactivated: usize) {
    let others = [uid(11), uid(12), uid(13)];
    let db = db_with_users(&[OWNER, others[0], others[1], others[2]]);
    block_on(db.insert_workspace(&new_ws(10, OWNER, Role::Guest, 0))).unwrap();
    seed_members(&db, wid(10), &others);
    for u in &others[..deactivated] {
        assert!(block_on(db.set_workspace_member_active(&sys(), wid(10), *u, false)).unwrap());
    }
    let mine = block_on(db.workspaces_of(OWNER)).unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!((mine[0].total_members, mine[0].role), (4 - deactivated as u64, Role::Guest));
    // an active member sees the same count
    let theirs = block_on(db.workspaces_of(others[2])).unwrap();
    assert_eq!(theirs.iter().map(|l| l.total_members).collect::<Vec<_>>(), if deactivated == 3 { vec![] } else { vec![4 - deactivated as u64] });
}

// --- order ------------------------------------------------------------------------------------------------------

/// The list of workspaces is newest first, and two made in the same second are ordered by id, the larger first.
#[rstest]
fn workspaces_are_listed_newest_first_with_ties_by_id(#[values(-1, 0, 1)] second_after: i64, #[values(false, true)] second_id_larger: bool) {
    let db = db_with_users(&[OWNER]);
    let (first_id, second_id) = if second_id_larger { (10, 20) } else { (20, 10) };
    block_on(db.insert_workspace(&new_ws(first_id, OWNER, Role::Admin, 100))).unwrap();
    block_on(db.insert_workspace(&new_ws(second_id, OWNER, Role::Admin, 100 + second_after))).unwrap();
    let got: Vec<u128> = block_on(db.workspaces_of(OWNER)).unwrap().iter().map(|l| l.workspace.id.0.as_u128()).collect();
    let want = match second_after {
        1 => vec![second_id, first_id],
        -1 => vec![first_id, second_id],
        _ => vec![first_id.max(second_id), first_id.min(second_id)],
    };
    assert_eq!(got, want);
}

/// The member list is every member row, deactivated too, newest first, with ties by the membership's id: the owner (made
/// later than the seeded rows, which share one timestamp) first, then the seeded rows by id, the larger first.
#[test]
fn the_member_list_is_newest_first_with_ties_by_membership_id_and_shows_the_person() {
    let others = [uid(11), uid(12), uid(13)];
    let db = db_with_users(&[OWNER, others[0], others[1], others[2]]);
    block_on(db.insert_workspace(&new_ws(10, OWNER, Role::Admin, 100))).unwrap();
    seed_members(&db, wid(10), &others);
    assert!(block_on(db.set_workspace_member_active(&sys(), wid(10), others[1], false)).unwrap());

    let listed = block_on(db.workspace_members(&scope(wid(10), OWNER))).unwrap();
    let ids: Vec<Uuid> = listed.iter().map(|m| m.member.id).collect();
    let mut seeded: Vec<Uuid> = others.iter().map(|u| block_on(db.workspace_member(&scope(wid(10), *u))).unwrap().unwrap().id).collect();
    seeded.sort_by(|a, b| b.cmp(a));
    let want: Vec<Uuid> = std::iter::once(Uuid::from_u128(0x1000 + 10)).chain(seeded).collect();
    assert_eq!(ids, want);
    let active: Vec<(UserId, bool)> = listed.iter().map(|m| (m.member.user_id, m.member.is_active)).collect();
    assert!(active.contains(&(others[1], false)), "{active:?}");
    // the person behind each row
    assert!(listed.iter().all(|m| m.user.id == m.member.user_id));
    // another workspace's members are not listed
    block_on(db.insert_workspace(&new_ws(20, others[0], Role::Admin, 0))).unwrap();
    assert_eq!(block_on(db.workspace_members(&scope(wid(20), others[0]))).unwrap().len(), 1);
}

// --- project roles ----------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RoleRow {
    /// The workspace the project belongs to (0 or 1).
    project_ws: usize,
    /// For each of the two users, the workspace their member row names (0 or 1) and the role, if they have a row at all.
    rows: [Option<(usize, Role)>; 2],
}

fn any_role() -> impl Strategy<Value = Role> {
    prop_oneof![Just(Role::Guest), Just(Role::Member), Just(Role::Admin)]
}

fn role_row() -> impl Strategy<Value = RoleRow> {
    let row = || prop::option::of((0..2usize, any_role()));
    (0..2usize, row(), row()).prop_map(|(project_ws, a, b)| RoleRow { project_ws, rows: [a, b] })
}

proptest! {
    #![proptest_config(proptest_config(128))]

    /// `project_roles` is the caller's member rows whose project really is in the workspace the row names, for the
    /// workspace of the scope, ordered by project id. Not another user's rows, not another workspace's, and not a row that
    /// names a workspace its project is not in.
    #[test]
    fn prop_project_roles_are_the_callers_consistent_rows_in_the_scope_workspace_by_project_id(
        projects in prop::collection::vec(role_row(), 0..7),
        caller in 0..2usize,
        in_ws in 0..2usize,
    ) {
        let users = [OWNER, OTHER];
        let ws = [wid(10), wid(11)];
        let db = db_with_users(&users);
        let f = SeedFixture {
            workspaces: (0..2).map(|i| SeedWorkspace { id: ws[i], slug: WorkspaceSlug::new(format!("w{i}")) }).collect(),
            // project ids are not in creation order, so that sorting by id is not the order they were written in
            projects: projects
                .iter()
                .enumerate()
                .map(|(i, p)| SeedProject { id: ProjectId(Uuid::from_u128(0xF000 - 7 * i as u128)), workspace_id: ws[p.project_ws], name: format!("p{i}") })
                .collect(),
            memberships: projects
                .iter()
                .enumerate()
                .flat_map(|(i, p)| {
                    p.rows.iter().enumerate().filter_map(move |(u, row)| {
                        row.map(|(row_ws, role)| SeedMembership { project_id: ProjectId(Uuid::from_u128(0xF000 - 7 * i as u128)), workspace_id: ws[row_ws], user_id: users[u], role })
                    })
                })
                .collect(),
            ..SeedFixture::default()
        };
        block_on(db.seed(&sys(), &f)).unwrap();

        let mut want: Vec<(ProjectId, Role)> = projects
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let (row_ws, role) = p.rows[caller]?;
                (row_ws == in_ws && p.project_ws == in_ws).then_some((ProjectId(Uuid::from_u128(0xF000 - 7 * i as u128)), role))
            })
            .collect();
        want.sort_by_key(|(p, _)| p.0);
        let got = block_on(db.project_roles(&WorkspaceScope::mint(ws[in_ws], users[caller], Role::Member))).unwrap();
        prop_assert_eq!(got, want);
    }
}
