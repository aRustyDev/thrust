//! The tenant key in the memory adapter (issues #35, #36): `membership` resolves the route's workspace slug, the
//! caller's membership of that workspace and of the project, and a member row written with another workspace does
//! not count; `allocate` refuses a project outside the workspace it is asked under. The Postgres adapter is held to
//! the same answers by the generated sequences in `thrust-adapters-selfhosted/tests/model`. Exhaustive over the
//! fixture's finite domain: every project, the slug named for it, and every user.

use futures_executor::block_on;
use rstest::rstest;
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::*;
use thrust_ports::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum P {
    A,
    B,
}

/// The workspace a call names for a project: its own, or the other tenant's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ws {
    Own,
    Other,
}

/// The slug a membership call names for a project: its own workspace's, the other one's, or one that names none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum S {
    Own,
    Other,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum U {
    AdminA,
    /// In workspace A, member of no project.
    OutsiderA,
    AdminB,
    /// In both workspaces, with a stray member row for project A that carries workspace B.
    StrayB,
    /// In both workspaces, Admin of both projects: one user, two workspaces.
    Dual,
}

const WS: [WorkspaceId; 2] = [WorkspaceId(Uuid::from_u128(1)), WorkspaceId(Uuid::from_u128(2))];
const SLUG: [&str; 2] = ["ws-a", "ws-b"];
const PROJECT: [ProjectId; 2] = [ProjectId(Uuid::from_u128(11)), ProjectId(Uuid::from_u128(12))];

fn slot(p: P) -> usize {
    match p {
        P::A => 0,
        P::B => 1,
    }
}

fn ws_for(p: P, ws: Ws) -> WorkspaceId {
    WS[if ws == Ws::Own { slot(p) } else { 1 - slot(p) }]
}

fn slug_for(p: P, s: S) -> WorkspaceSlug {
    match s {
        S::Own => WorkspaceSlug::new(SLUG[slot(p)]),
        S::Other => WorkspaceSlug::new(SLUG[1 - slot(p)]),
        S::Unknown => WorkspaceSlug::new("no-such-workspace"),
    }
}

fn user(u: U) -> UserId {
    UserId(Uuid::from_u128(21 + u as u128))
}

fn seeded() -> MemoryDatabase {
    let db = MemoryDatabase::new();
    let m = |project, workspace_id, u| SeedMembership { project_id: PROJECT[project], workspace_id, user_id: user(u), role: Role::Admin };
    let in_ws = |ws: usize, u| SeedWorkspaceMember { workspace_id: WS[ws], user_id: user(u) };
    block_on(db.seed(&SystemScope::mint(), &SeedFixture {
        workspaces: (0..2).map(|i| SeedWorkspace { id: WS[i], slug: WorkspaceSlug::new(SLUG[i]) }).collect(),
        users: [U::AdminA, U::OutsiderA, U::AdminB, U::StrayB, U::Dual].map(user).to_vec(),
        workspace_members: vec![
            in_ws(0, U::AdminA),
            in_ws(0, U::OutsiderA),
            in_ws(1, U::AdminB),
            in_ws(0, U::StrayB),
            in_ws(1, U::StrayB),
            in_ws(0, U::Dual),
            in_ws(1, U::Dual),
        ],
        projects: vec![
            SeedProject { id: PROJECT[0], workspace_id: WS[0], name: "A".into() },
            SeedProject { id: PROJECT[1], workspace_id: WS[1], name: "B".into() },
        ],
        memberships: vec![
            m(0, WS[0], U::AdminA),
            m(1, WS[1], U::AdminB),
            m(0, WS[1], U::StrayB),
            m(0, WS[0], U::Dual),
            m(1, WS[1], U::Dual),
        ],
        sessions: vec![],
    }))
    .unwrap();
    db
}

/// Which workspaces a user belongs to, as `[A, B]`; written out as the fixture's spec.
fn in_workspace(u: U) -> [bool; 2] {
    match u {
        U::AdminA | U::OutsiderA => [true, false],
        U::AdminB => [false, true],
        U::StrayB | U::Dual => [true, true],
    }
}

/// Which projects a user is an Admin of, as `[A, B]`. `StrayB`'s row for project A names workspace B, so it is not
/// a membership of project A.
fn admin_of(u: U) -> [bool; 2] {
    match u {
        U::AdminA => [true, false],
        U::AdminB => [false, true],
        U::Dual => [true, true],
        U::OutsiderA | U::StrayB => [false, false],
    }
}

#[rstest]
fn membership_resolves_the_slug_the_workspace_and_only_member_rows_of_the_projects_workspace(
    #[values(P::A, P::B)] p: P,
    #[values(S::Own, S::Other, S::Unknown)] s: S,
    #[values(U::AdminA, U::OutsiderA, U::AdminB, U::StrayB, U::Dual)] u: U,
) {
    let want = match s {
        // a slug that names no workspace, or another workspace than the project's, shows nothing
        S::Unknown | S::Other => Membership::NoSuchProject,
        S::Own if !in_workspace(u)[slot(p)] => Membership::NoSuchProject,
        S::Own if admin_of(u)[slot(p)] => Membership::Member { workspace_id: WS[slot(p)], role: Role::Admin },
        S::Own => Membership::NotAMember,
    };
    let got = block_on(seeded().membership(&slug_for(p, s), PROJECT[slot(p)], user(u))).unwrap();
    assert_eq!(got, want, "{u:?} on {p:?} via {s:?}");
}

/// `authorize` never mints a scope whose project is outside its workspace; the test does, to reach the adapter's own
/// check.
#[rstest]
fn allocate_refuses_a_project_outside_the_workspace(#[values(P::A, P::B)] p: P, #[values(Ws::Own, Ws::Other)] ws: Ws) {
    let db = seeded();
    let scope = Scope::mint(ws_for(p, ws), PROJECT[slot(p)], user(U::AdminA));
    let mut tx = block_on(db.begin(&scope)).unwrap();
    match (ws, block_on(tx.allocate(&scope))) {
        (Ws::Own, Ok(a)) => assert_eq!(a, Allocation { sequence_id: 1, sort_order: 10_000 }),
        (Ws::Other, Err(DbError::Backend(_))) => {}
        (ws, got) => panic!("allocate under {ws:?} workspace: got {got:?}"),
    }
}
