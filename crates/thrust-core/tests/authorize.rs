//! The permission seam and the queue consumer, against the memory adapters.

mod common;

use common::*;
use futures_executor::block_on;
use rstest::rstest;
use thrust_contracts::chrono::TimeDelta;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::NewSession;

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
    let got = block_on(authorize(&w.app, &who, &w.slug_a, w.project_a, perm));
    let want = if allowed(role, perm) { Outcome::Ok } else { Outcome::Forbidden };
    assert_eq!(outcome(&got), want, "{actor:?} {perm:?}");
    if let Ok(scope) = got {
        // the proof value carries the caller's tenant, project and identity, not anyone else's
        assert_eq!((scope.workspace_id(), scope.project_id(), scope.user_id()), (w.ws_a, w.project_a, who.user_id));
    }
}

/// Insert a session row for `who` under `token`: created `age_secs` before the fixed clock's instant, expiring
/// `expires_in_secs` after it (negative: already expired).
fn insert_session(w: &World, token: &str, who: &Principal, age_secs: i64, expires_in_secs: i64) {
    let now = w.app.ports.clock.now();
    let row = NewSession {
        token_hash: hash_token(token),
        user_id: who.user_id,
        created_at: now - TimeDelta::seconds(age_secs),
        expires_at: now + TimeDelta::seconds(expires_in_secs),
    };
    block_on(w.app.ports.db.insert_session(&row)).unwrap();
}

#[derive(Debug, Clone, Copy)]
enum Life {
    Live,
    /// `expires_at` is exactly now: expired (the row is live only while `expires_at > now`).
    ExpiresNow,
    Expired,
}

/// A bearer token resolves to a user exactly when its row exists, is not revoked and is not expired, for every
/// combination of those. The principal is the user and nothing else (no workspace).
#[rstest]
fn authenticate_resolves_a_live_session_and_nothing_else(
    #[values(true, false)] known: bool,
    #[values(Life::Live, Life::ExpiresNow, Life::Expired)] life: Life,
    #[values(false, true)] revoked: bool,
) {
    let w = world();
    let who = w.principal(Actor::MemberA);
    let expires_in = match life {
        Life::Live => 60,
        Life::ExpiresNow => 0,
        Life::Expired => -60,
    };
    insert_session(&w, "tok-1", &who, 10, expires_in);
    if revoked {
        block_on(revoke_session(&w.app, "tok-1")).unwrap();
    }
    let got = block_on(authenticate(&w.app, if known { "tok-1" } else { "tok-unknown" }));
    if known && matches!(life, Life::Live) && !revoked {
        assert_eq!(got.unwrap(), who);
    } else {
        assert!(matches!(got, Err(CoreError::Unauthenticated)), "known={known} {life:?} revoked={revoked}: {got:?}");
    }
}

/// `last_seen_at` is written lazily: only when it is at least an hour old (the spec's number, written out rather
/// than read from `CoreConfig`). A fresh session costs a request no write.
#[rstest]
fn authenticate_touches_a_session_only_when_it_is_stale(
    #[values(0, 3_599, 3_600, 86_400)] age_secs: i64,
) {
    let w = world();
    let who = w.principal(Actor::MemberA);
    insert_session(&w, "tok-1", &who, age_secs, 3_600_000);
    let now = w.app.ports.clock.now();
    block_on(authenticate(&w.app, "tok-1")).unwrap();
    let seen = block_on(w.app.ports.db.session(&hash_token("tok-1"), now)).unwrap().unwrap().last_seen_at;
    let want = if age_secs >= 3_600 { now } else { now - TimeDelta::seconds(age_secs) };
    assert_eq!(seen, want, "a session last seen {age_secs} s ago");
}

/// Sign-out is idempotent and never resurrects a session: a second revoke keeps the first revocation.
#[test]
fn revoking_a_session_twice_leaves_it_revoked() {
    let w = world();
    let who = w.principal(Actor::MemberA);
    insert_session(&w, "tok-1", &who, 10, 3_600);
    block_on(authenticate(&w.app, "tok-1")).unwrap();
    block_on(revoke_session(&w.app, "tok-1")).unwrap();
    block_on(revoke_session(&w.app, "tok-1")).unwrap();
    block_on(revoke_session(&w.app, "tok-never-issued")).unwrap();
    assert!(matches!(block_on(authenticate(&w.app, "tok-1")), Err(CoreError::Unauthenticated)));
}

/// Pinned: the NIST FIPS 180-4 example for SHA-256("abc"). What the database stores for a token is this digest.
#[test]
fn hash_token_is_sha256() {
    let want = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23, 0xb0, 0x03,
        0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
    ];
    assert_eq!(hash_token("abc").0, want);
    assert_ne!(hash_token("abd"), hash_token("abc"));
}

#[test]
fn a_workspace_member_outside_the_project_is_forbidden() {
    let w = world();
    let got = block_on(authorize(&w.app, &w.principal(Actor::OutsiderA), &w.slug_a, w.project_a, Permission::ReadWorkItems));
    assert_eq!(outcome(&got), Outcome::Forbidden);
}

/// Cross-tenant probes learn nothing: another tenant's project looks exactly like a project that does not exist.
#[rstest]
fn another_tenants_project_is_not_found(
    #[values(Permission::ReadWorkItems, Permission::CreateWorkItem, Permission::ReadDeleted)] perm: Permission,
) {
    let w = world();
    let cross = block_on(authorize(&w.app, &w.principal(Actor::MemberB), &w.slug_a, w.project_a, perm));
    let missing = block_on(authorize(&w.app, &w.principal(Actor::MemberB), &w.slug_b, ProjectId(Uuid::from_u128(0xDEAD)), perm));
    assert_eq!(outcome(&cross), Outcome::NotFound);
    assert_eq!(outcome(&missing), Outcome::NotFound);
}

/// Who belongs to which workspace and project, written out as the fixture's spec: `([in A, in B], [role in
/// project A, role in project B])`. Read it with `common::world`, not from the code under test.
fn standing(actor: Actor) -> ([bool; 2], [Option<Role>; 2]) {
    match actor {
        Actor::AdminA => ([true, false], [Some(Role::Admin), None]),
        Actor::MemberA => ([true, false], [Some(Role::Member), None]),
        Actor::GuestA => ([true, false], [Some(Role::Guest), None]),
        Actor::OutsiderA => ([true, false], [None, None]),
        Actor::MemberB => ([false, true], [None, Some(Role::Member)]),
        Actor::Dual => ([true, true], [Some(Role::Member), Some(Role::Guest)]),
    }
}

fn slot(p: Proj) -> usize {
    match p {
        Proj::A => 0,
        Proj::B => 1,
    }
}

/// The slug picks the workspace the project is looked up in (ADR-0051 round 29), for every actor, project and
/// slug: a slug that names no workspace, a workspace the caller is not in and a project that is not in the named
/// workspace are all Not Found; a workspace member who is not a member of the project is Forbidden; and one
/// session (`Actor::Dual`) works in both workspaces.
#[rstest]
fn the_slug_decides_which_workspace_a_project_is_looked_up_in(
    #[values(Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::MemberB, Actor::Dual)] actor: Actor,
    #[values(Proj::A, Proj::B)] proj: Proj,
    #[values(Slug::Own, Slug::Other, Slug::Unknown)] slug: Slug,
) {
    let w = world();
    let got = block_on(authorize(
        &w.app,
        &w.principal(actor),
        &w.slug(proj, slug),
        w.project(proj),
        Permission::ReadWorkItems,
    ));
    let (in_ws, role) = standing(actor);
    let want = match slug {
        Slug::Unknown | Slug::Other => Outcome::NotFound,
        Slug::Own if !in_ws[slot(proj)] => Outcome::NotFound,
        Slug::Own if role[slot(proj)].is_none() => Outcome::Forbidden,
        Slug::Own => Outcome::Ok,
    };
    assert_eq!(outcome(&got), want, "{actor:?} on {proj:?} via {slug:?}");
    if let Ok(scope) = got {
        // the scope names the workspace the slug resolved to, which is the project's own
        assert_eq!((scope.workspace_id(), scope.project_id()), (w.workspace_of(proj), w.project(proj)));
    }
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
    let rows = block_on(list_activity(&w.app, &w.principal(Actor::GuestA), &w.slug_a, w.project_a, None)).unwrap();
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
