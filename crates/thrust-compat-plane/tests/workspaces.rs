//! What the workspace routes derive from stored rows that the recorded journey never has: a member who is not an admin, a
//! workspace with projects, a member who has been deactivated. The pin's answers for those come from its source (the
//! journey has one admin and no project), so these tests state the rules instead of replaying a recording.
//!
//! What must hold: a member row names its role with Plane's numbers (guest 5, member 15, admin 20); a guest's member list
//! carries no email and no sign-in method, everyone else's carries both; `project-roles` holds exactly the caller's roles in
//! the projects of THIS workspace, keyed by project id, whatever other workspaces and projects exist; and a deactivated
//! member is listed as suspended to the others, finds the workspace gone from their own list, and is refused its members
//! and its project roles.

mod common;

use axum::Router;
use common::*;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Value};
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;

const STRONG: &str = "Workspaces-Sample-Pa55word-Zebra-81";

/// A world with a signed-up account for each email, and the jar that holds their sessions.
fn world(emails: &[&str]) -> (Router, std::sync::Arc<MemoryDatabase>, Jar) {
    let (router, db) = app_with_db();
    let mut jar = Jar::new();
    for email in emails {
        let raw = send(&router, &json!("sign-up"), &json!({"method": "POST", "path": "/auth/sign-up/", "body": {"email": email, "password": STRONG}}), &jar);
        assert_eq!(raw["response"]["status"], 200, "{raw}");
        jar.insert(jar_key(email), session_token(&raw).expect("a session"));
    }
    (router, db, jar)
}

fn call(router: &Router, jar: &Jar, who: &str, method: &str, path: &str, body: Value) -> Value {
    let mut req = json!({"method": method, "path": path, "as": who});
    if !body.is_null() {
        req["body"] = body;
    }
    send(router, &json!("call"), &req, jar)["response"].clone()
}

fn user_id(db: &MemoryDatabase, email: &str) -> UserId {
    futures_executor::block_on(db.user_by_email(email)).unwrap().expect("the account exists").id
}

fn at(n: i64) -> Timestamp {
    DateTime::from_timestamp(1_760_000_000 + n, 0).unwrap()
}

/// A workspace whose only member is `user`, with `role`.
fn make_workspace(db: &MemoryDatabase, id: u128, slug: &str, user: UserId, role: Role) -> WorkspaceId {
    let id = WorkspaceId(Uuid::from_u128(id));
    let workspace = Workspace {
        id,
        slug: WorkspaceSlug::new(slug),
        name: slug.to_owned(),
        owner_id: Some(user),
        organization_size: None,
        timezone: "UTC".into(),
        background_color: "#808080".into(),
        created_at: at(0),
        updated_at: at(0),
    };
    let owner = WorkspaceMember {
        id: Uuid::from_u128(id.0.as_u128() ^ 0xFFFF),
        workspace_id: id,
        user_id: user,
        role,
        company_role: None,
        is_active: true,
        props: json!({}),
        created_at: at(0),
        updated_at: at(0),
    };
    futures_executor::block_on(db.insert_workspace(&NewWorkspace { workspace, owner })).unwrap();
    id
}

fn seed(db: &MemoryDatabase, f: SeedFixture) {
    futures_executor::block_on(db.seed(&SystemScope::mint(), &f)).unwrap();
}

fn role_number(r: Role) -> u8 {
    match r {
        Role::Guest => 5,
        Role::Member => 15,
        Role::Admin => 20,
    }
}

fn any_role() -> impl Strategy<Value = Role> {
    prop_oneof![Just(Role::Guest), Just(Role::Member), Just(Role::Admin)]
}

/// The member list as seen by `caller` of a workspace that has the caller (with `role`) and one other member (a member).
fn members_seen_by(role: Role) -> Value {
    let (router, db, jar) = world(&["caller@example.test", "other@example.test"]);
    let (caller, other) = (user_id(&db, "caller@example.test"), user_id(&db, "other@example.test"));
    let ws = make_workspace(&db, 0xA1, "seen-by", caller, role);
    seed(&db, SeedFixture { workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: other }], ..SeedFixture::default() });
    let r = call(&router, &jar, "caller@example.test", "GET", "/api/workspaces/seen-by/members/", Value::Null);
    assert_eq!(r["status"], 200, "{r}");
    r["body"].clone()
}

/// The spec's table: whether the member list shows the person's email and sign-in method to a caller of this role.
fn shows_contact_details(caller: Role) -> bool {
    caller != Role::Guest
}

#[rstest]
fn the_member_list_names_roles_with_plane_numbers_and_hides_contact_details_from_a_guest(
    #[values(Role::Guest, Role::Member, Role::Admin)] caller: Role,
) {
    let rows = members_seen_by(caller);
    let rows = rows.as_array().expect("a list");
    assert_eq!(rows.len(), 2, "{rows:?}");
    // the caller's own row carries the caller's role, the other member's is a member
    let numbers: Vec<u64> = rows.iter().map(|r| r["role"].as_u64().unwrap()).collect();
    assert!(numbers.contains(&u64::from(role_number(caller))), "{numbers:?}");
    assert!(numbers.contains(&15), "{numbers:?}");
    for row in rows {
        let person = row["member"].as_object().expect("the person");
        assert_eq!(person.contains_key("email"), shows_contact_details(caller), "email for a {caller:?}: {person:?}");
        assert_eq!(person.contains_key("last_login_medium"), shows_contact_details(caller), "medium for a {caller:?}: {person:?}");
        // what everyone sees
        for key in ["id", "first_name", "last_name", "avatar", "avatar_url", "display_name", "is_bot"] {
            assert!(person.contains_key(key), "{key} for a {caller:?}: {person:?}");
        }
    }
}

proptest! {
    #![proptest_config(proptest_config(32))]

    /// `project-roles` is `{project id: role number}` for the caller's projects in this workspace and no others: not the
    /// projects of another workspace the caller is in, not another person's roles here, and not a row that names this
    /// workspace for a project that belongs to another.
    #[test]
    fn prop_project_roles_are_the_callers_roles_in_this_workspace_by_project(
        mine in prop::collection::vec(any_role(), 0..5),
        theirs in prop::collection::vec(any_role(), 0..3),
        others_here in prop::collection::vec(any_role(), 0..3),
    ) {
        let (router, db, jar) = world(&["me@example.test", "someone@example.test"]);
        let (me, someone) = (user_id(&db, "me@example.test"), user_id(&db, "someone@example.test"));
        let here = make_workspace(&db, 0xB1, "roles-here", me, Role::Member);
        let there = make_workspace(&db, 0xB2, "roles-there", me, Role::Member);
        let project = |n: u128| ProjectId(Uuid::from_u128(0xC000 + n));
        let mut want = serde_json::Map::new();
        let mut f = SeedFixture::default();
        for (i, role) in mine.iter().enumerate() {
            let p = project(i as u128);
            f.projects.push(SeedProject { id: p, workspace_id: here, name: format!("p{i}") });
            f.memberships.push(SeedMembership { project_id: p, workspace_id: here, user_id: me, role: *role });
            want.insert(p.0.to_string(), json!(role_number(*role)));
        }
        for (i, role) in theirs.iter().enumerate() {
            let p = project(100 + i as u128);
            f.projects.push(SeedProject { id: p, workspace_id: there, name: format!("t{i}") });
            f.memberships.push(SeedMembership { project_id: p, workspace_id: there, user_id: me, role: *role });
        }
        for (i, role) in others_here.iter().enumerate() {
            // someone else's role in a project of this workspace: not mine
            if i < mine.len() {
                f.memberships.push(SeedMembership { project_id: project(i as u128), workspace_id: here, user_id: someone, role: *role });
            }
        }
        // a stray row: a project of the other workspace, written with this one's id
        f.memberships.push(SeedMembership { project_id: project(100), workspace_id: here, user_id: me, role: Role::Admin });
        seed(&db, f);
        let r = call(&router, &jar, "me@example.test", "GET", "/api/users/me/workspaces/roles-here/project-roles/", Value::Null);
        prop_assert_eq!(&r["status"], 200, "{}", r);
        prop_assert_eq!(&r["body"], &Value::Object(want));
    }
}

const BOTH_REQUIRED: &str = "Both name and slug are required";

/// The pin's `if not name or not slug` is Python's truthiness, whatever JSON type the field has: null, false, zero, an empty
/// string, list or object are "missing"; anything else is given, a number included. (The spec is Python's, so the table is
/// written out.)
#[rstest]
fn the_both_required_check_follows_pythons_truthiness(
    #[values(
        (json!(null), false), (json!(false), false), (json!(0), false), (json!(0.0), false), (json!(""), false),
        (json!([]), false), (json!({}), false),
        (json!(true), true), (json!(1), true), (json!(-2), true), (json!(0.5), true), (json!("x"), true), (json!(" "), true),
        (json!([0]), true), (json!({"a": 1}), true)
    )]
    field: (Value, bool),
    #[values(true, false)] in_name: bool,
) {
    let (value, given) = field;
    let (router, _, jar) = world(&["truthy@example.test"]);
    let body = if in_name { json!({"name": value, "slug": "truthy-slug"}) } else { json!({"name": "Truthy name", "slug": value}) };
    let r = call(&router, &jar, "truthy@example.test", "POST", "/api/workspaces/", body);
    let refused_as_missing = r["status"] == 400 && r["body"] == json!({"error": BOTH_REQUIRED});
    assert_eq!(refused_as_missing, !given, "{value} in the {}: {r}", if in_name { "name" } else { "slug" });
}

/// A name may have 80 characters and a slug 48, counted as characters and not bytes: one more of either is refused with the
/// view's `error`, and a name of two-byte letters is as long as its letters.
#[rstest]
fn the_name_and_slug_lengths_are_limited_to_80_and_48_characters(
    #[values(79, 80, 81)] name_len: usize,
    #[values(47, 48, 49)] slug_len: usize,
    #[values("x", "é")] letter: &str,
) {
    let (router, _, jar) = world(&["limits@example.test"]);
    let body = json!({"name": letter.repeat(name_len), "slug": "a".repeat(slug_len)});
    let r = call(&router, &jar, "limits@example.test", "POST", "/api/workspaces/", body);
    if name_len > 80 || slug_len > 48 {
        assert_eq!((&r["status"], &r["body"]), (&json!(400), &json!({"error": "The maximum length for name is 80 and for slug is 48"})), "{r}");
    } else {
        assert_eq!(r["status"], 201, "{r}");
    }
}

/// A member who is deactivated keeps the row, so the others see them as suspended, but the workspace is gone from their
/// own list and every route that names it refuses them (the member list and the project roles) or answers blank (their own
/// member row), as the pin does for a person who is not a member.
#[test]
fn a_deactivated_member_is_listed_as_suspended_and_loses_the_workspace() {
    let (router, db, jar) = world(&["admin@example.test", "gone@example.test"]);
    let (admin, gone) = (user_id(&db, "admin@example.test"), user_id(&db, "gone@example.test"));
    let ws = make_workspace(&db, 0xD1, "suspend", admin, Role::Admin);
    seed(&db, SeedFixture { workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: gone }], ..SeedFixture::default() });
    let get = |who: &str, path: &str| call(&router, &jar, who, "GET", path, Value::Null);
    let listed = |who: &str| get(who, "/api/users/me/workspaces/")["body"].as_array().unwrap().len();
    assert_eq!(listed("gone@example.test"), 1, "control: before the change the member has the workspace");
    assert_eq!(get("gone@example.test", "/api/workspaces/suspend/members/")["status"], 200, "control: and its member list");

    assert!(futures_executor::block_on(db.set_workspace_member_active(&SystemScope::mint(), ws, gone, false)).unwrap());

    assert_eq!(listed("gone@example.test"), 0);
    assert_eq!(listed("admin@example.test"), 1, "the others keep it");
    let refused = get("gone@example.test", "/api/workspaces/suspend/members/");
    assert_eq!(refused["status"], 403, "{refused}");
    let refused = get("gone@example.test", "/api/users/me/workspaces/suspend/project-roles/");
    assert_eq!(refused["status"], 403, "{refused}");
    let blank = get("gone@example.test", "/api/workspaces/suspend/workspace-members/me/");
    assert_eq!((&blank["status"], &blank["body"]["is_active"], &blank["body"]["member"]), (&json!(200), &json!(false), &Value::Null), "{blank}");

    let seen = get("admin@example.test", "/api/workspaces/suspend/members/");
    let rows = seen["body"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "the suspended member stays in the list: {rows:?}");
    let active: Vec<bool> = rows.iter().map(|r| r["is_active"].as_bool().unwrap()).collect();
    assert_eq!(active.iter().filter(|a| **a).count(), 1, "{active:?}");
    // the workspace counts only the active members
    let mine = get("admin@example.test", "/api/users/me/workspaces/");
    assert_eq!(mine["body"][0]["total_members"], 1, "{mine}");
}
