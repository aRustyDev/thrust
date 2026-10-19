//! The permission seam and the queue consumer, against the memory adapters.

mod common;

use common::*;
use futures_executor::block_on;
use rstest::rstest;
use thrust_contracts::*;
use thrust_core::*;

/// The role a permission needs, written out as the spec (Plane: Guest < Member < Admin) rather than read from
/// `Permission::min_role`, so the table and the code can disagree.
fn allowed(role: Role, perm: Permission) -> bool {
    match perm {
        Permission::ReadWorkItems | Permission::ReadAttachment => true,
        Permission::CreateWorkItem | Permission::DeleteWorkItem | Permission::WriteAttachment => role != Role::Guest,
        Permission::ReadDeleted => role == Role::Admin,
    }
}

#[rstest]
fn authorize_follows_the_role_table(
    #[values(Actor::GuestA, Actor::MemberA, Actor::AdminA)] actor: Actor,
    #[values(
        Permission::ReadWorkItems,
        Permission::ReadDeleted,
        Permission::CreateWorkItem,
        Permission::DeleteWorkItem,
        Permission::ReadAttachment,
        Permission::WriteAttachment
    )]
    perm: Permission,
) {
    let w = world();
    let role = match actor {
        Actor::GuestA => Role::Guest,
        Actor::MemberA => Role::Member,
        _ => Role::Admin,
    };
    let who = w.principal(actor);
    let got = block_on(authorize(&w.app, &who, w.project_a, perm));
    let want = if allowed(role, perm) { Outcome::Ok } else { Outcome::Forbidden };
    assert_eq!(outcome(&got), want, "{actor:?} {perm:?}");
    if let Ok(scope) = got {
        // the proof value carries the caller's tenant, project and identity, not anyone else's
        assert_eq!((scope.workspace_id(), scope.project_id(), scope.user_id()), (w.ws_a, w.project_a, who.user_id));
    }
}

/// cargo-mutants cannot mutate `authenticate` (its only mutant does not compile), so this test is its only check.
#[test]
fn authenticate_resolves_a_live_session_and_nothing_else() {
    let w = world();
    let who = w.principal(Actor::MemberA);
    block_on(w.app.ports.sessions.put("tok-1", &who, 60)).unwrap();
    assert_eq!(block_on(authenticate(&w.app, "tok-1")).unwrap(), who);
    let unknown = block_on(authenticate(&w.app, "tok-unknown"));
    assert!(matches!(unknown, Err(CoreError::Unauthenticated)), "{unknown:?}");
    block_on(w.app.ports.sessions.put("tok-expired", &who, 0)).unwrap();
    let expired = block_on(authenticate(&w.app, "tok-expired"));
    assert!(matches!(expired, Err(CoreError::Unauthenticated)), "an expired session is not a principal: {expired:?}");
}

#[test]
fn a_workspace_member_outside_the_project_is_forbidden() {
    let w = world();
    let got = block_on(authorize(&w.app, &w.principal(Actor::OutsiderA), w.project_a, Permission::ReadWorkItems));
    assert_eq!(outcome(&got), Outcome::Forbidden);
}

/// Cross-tenant probes learn nothing: another tenant's project looks exactly like a project that does not exist.
#[rstest]
fn another_tenants_project_is_not_found(
    #[values(Permission::ReadWorkItems, Permission::CreateWorkItem, Permission::ReadDeleted)] perm: Permission,
) {
    let w = world();
    let cross = block_on(authorize(&w.app, &w.principal(Actor::MemberB), w.project_a, perm));
    let missing = block_on(authorize(&w.app, &w.principal(Actor::MemberB), ProjectId(Uuid::from_u128(0xDEAD)), perm));
    assert_eq!(outcome(&cross), Outcome::NotFound);
    assert_eq!(outcome(&missing), Outcome::NotFound);
}

fn created_event(w: &World) -> Envelope {
    Envelope {
        event_id: EventId(Uuid::from_u128(0xE1)),
        schema_version: 1,
        workspace_id: w.ws_a,
        event_type: "work_item.created".into(),
        entity_type: "work_item".into(),
        entity_id: Uuid::from_u128(0xE2),
        actor_id: Some(w.principal(Actor::MemberA).user_id),
        occurred_at: w.app.ports.clock.now(),
        payload: serde_json::json!({ "project_id": w.project_a }),
        trace_id: None,
    }
}

/// At-least-once delivery: a redelivered event must not write a second activity row.
#[test]
fn process_event_is_idempotent_on_event_id() {
    let w = world();
    let env = created_event(&w);
    assert!(block_on(process_event(&w.app, &env)).unwrap(), "first delivery inserts");
    assert!(!block_on(process_event(&w.app, &env)).unwrap(), "redelivery inserts nothing");
    let rows = block_on(list_activity(&w.app, &w.principal(Actor::GuestA), w.project_a, None)).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].verb, "created");
    assert_eq!(rows[0].event_id, env.event_id);
}

#[test]
fn an_event_without_a_project_id_is_invalid() {
    let w = world();
    let mut env = created_event(&w);
    env.payload = serde_json::json!({});
    assert_eq!(outcome(&block_on(process_event(&w.app, &env))), Outcome::Invalid);
}
