//! The tenant key in the memory adapter (issue #35): `allocate` refuses a project outside the workspace it is asked
//! under, and a member row written with another workspace does not count. The Postgres adapter is held to the same
//! answers by the generated sequences in `thrust-adapters-selfhosted/tests/model`. Exhaustive over the fixture's
//! finite domain: every project, the workspace named for it, and every user.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum U {
    AdminA,
    /// In workspace A, member of nothing.
    OutsiderA,
    AdminB,
    /// In workspace B, with a stray member row for project A that carries workspace B.
    StrayB,
}

const WS: [WorkspaceId; 2] = [WorkspaceId(Uuid::from_u128(1)), WorkspaceId(Uuid::from_u128(2))];
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

fn user(u: U) -> UserId {
    UserId(Uuid::from_u128(21 + u as u128))
}

fn seeded() -> MemoryDatabase {
    let db = MemoryDatabase::new();
    let m = |project, workspace_id, u| SeedMembership { project_id: PROJECT[project], workspace_id, user_id: user(u), role: Role::Admin };
    block_on(db.seed(&SeedFixture {
        workspaces: WS.to_vec(),
        users: [U::AdminA, U::OutsiderA, U::AdminB, U::StrayB].map(user).to_vec(),
        projects: vec![
            SeedProject { id: PROJECT[0], workspace_id: WS[0], name: "A".into() },
            SeedProject { id: PROJECT[1], workspace_id: WS[1], name: "B".into() },
        ],
        memberships: vec![m(0, WS[0], U::AdminA), m(1, WS[1], U::AdminB), m(0, WS[1], U::StrayB)],
        sessions: vec![],
    }))
    .unwrap();
    db
}

#[rstest]
fn membership_counts_only_member_rows_of_the_projects_workspace(
    #[values(P::A, P::B)] p: P,
    #[values(Ws::Own, Ws::Other)] ws: Ws,
    #[values(U::AdminA, U::OutsiderA, U::AdminB, U::StrayB)] u: U,
) {
    let want = match (ws, p, u) {
        (Ws::Other, _, _) => Membership::NoSuchProject,
        (Ws::Own, P::A, U::AdminA) | (Ws::Own, P::B, U::AdminB) => Membership::Member(Role::Admin),
        (Ws::Own, _, _) => Membership::NotAMember,
    };
    let got = block_on(seeded().membership(ws_for(p, ws), PROJECT[slot(p)], user(u))).unwrap();
    assert_eq!(got, want);
}

#[rstest]
fn allocate_refuses_a_project_outside_the_workspace(#[values(P::A, P::B)] p: P, #[values(Ws::Own, Ws::Other)] ws: Ws) {
    let db = seeded();
    let mut tx = block_on(db.begin()).unwrap();
    match (ws, block_on(tx.allocate(ws_for(p, ws), PROJECT[slot(p)]))) {
        (Ws::Own, Ok(a)) => assert_eq!(a, Allocation { sequence_id: 1, sort_order: 10_000 }),
        (Ws::Other, Err(DbError::Backend(_))) => {}
        (ws, got) => panic!("allocate under {ws:?} workspace: got {got:?}"),
    }
}
