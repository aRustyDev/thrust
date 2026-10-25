//! What the workspace-load and home routes (S17, S19 to S26) derive from rows and callers that the recorded journey never has: a
//! guest, a member who has been deactivated, a stranger, a workspace with projects. The pin's answers for those come from its
//! source (the journey has one admin and no project), so these tests state the rules instead of replaying a recording.
//!
//! What must hold: every row refuses no session with 401, and a stranger and a deactivated member with 403, the `{"error"}`
//! body for the views Plane guards with its decorator and the `{"detail"}` body for the two it guards with a permission class
//! (S20, S22); S19 alone refuses a guest; the project list names its projects with the 21 keys of the pin's `.values()` query
//! and, for each, the caller's role number, the network number, the archive time and the caller's own sort order, and lists
//! exactly the projects the caller's workspace role lets them see; and a first visit's rows are the same on the second.

mod common;

use common::*;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Value};
use thrust_contracts::chrono::SecondsFormat;
use thrust_contracts::*;
use thrust_ports::*;

/// The nine routes, by the row of the inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    S17,
    S19,
    S20,
    S21,
    S22,
    S23,
    S24,
    S25,
    S26,
}

impl Row {
    fn path(self, slug: &str) -> String {
        let tail = match self {
            Row::S17 => "projects/",
            Row::S19 => "user-favorites/",
            Row::S20 => "states/",
            Row::S21 => "sidebar-preferences/",
            Row::S22 => "user-properties/",
            Row::S23 => "users/notifications/unread/",
            Row::S24 => "home-preferences/",
            Row::S25 => "quick-links/",
            Row::S26 => "recent-visits/",
        };
        format!("/api/workspaces/{slug}/{tail}")
    }

    /// The 403 body of the row: S20 and S22 are guarded by a permission class, the others by the decorator.
    fn refusal(self) -> Value {
        match self {
            Row::S20 | Row::S22 => json!({"detail": "You do not have permission to perform this action."}),
            _ => json!({"error": "You don't have the required permissions."}),
        }
    }

    /// Whether a guest may ask: everyone who is in the workspace may, except for the starred list (admins and members only).
    fn guests_may_ask(self) -> bool {
        self != Row::S19
    }

    /// What a member's answer is made of, in outline: the exact content is the fixtures' and the tests below's.
    fn shape_holds(self, body: &Value) -> bool {
        match self {
            Row::S17 | Row::S19 | Row::S20 | Row::S25 | Row::S26 => body == &json!([]),
            Row::S21 => body.as_object().is_some_and(|o| o.len() == 7),
            Row::S22 => body["filters"].is_object() && body["display_filters"]["display_filters"].is_object(),
            Row::S23 => body == &json!({"total_unread_notifications_count": 0, "mention_unread_notifications_count": 0}),
            Row::S24 => body.as_array().is_some_and(|a| a.len() == 3),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Caller {
    Anonymous,
    /// Signed in, in no workspace of the workspace they ask about.
    Stranger,
    /// Was a member; no longer active.
    Deactivated,
    Guest,
    Member,
    Admin,
}

/// The spec's table, written out: what each caller is answered on each row.
fn expected(row: Row, caller: Caller) -> Option<(u16, Value)> {
    match caller {
        Caller::Anonymous => Some((401, json!({"detail": "Authentication credentials were not provided."}))),
        Caller::Stranger | Caller::Deactivated => Some((403, row.refusal())),
        Caller::Guest if !row.guests_may_ask() => Some((403, row.refusal())),
        Caller::Guest | Caller::Member | Caller::Admin => None,
    }
}

#[rstest]
fn who_may_ask_a_workspace_load_row_and_what_the_pin_answers_the_rest(
    #[values(Row::S17, Row::S19, Row::S20, Row::S21, Row::S22, Row::S23, Row::S24, Row::S25, Row::S26)] row: Row,
    #[values(Caller::Anonymous, Caller::Stranger, Caller::Deactivated, Caller::Guest, Caller::Member, Caller::Admin)] caller: Caller,
) {
    let emails = ["owner@example.test", "stranger@example.test", "deactivated@example.test", "guest@example.test", "member@example.test", "admin@example.test"];
    let (router, db, jar) = world(&emails);
    let id = |e: &str| user_id(&db, e);
    // a workspace for each way of being in one, and one that only its owner is in
    make_workspace(&db, 0xA0, "owned", id(emails[0]), Role::Admin);
    make_workspace(&db, 0xA1, "deactivated-in", id(emails[2]), Role::Member);
    futures_executor::block_on(db.set_workspace_member_active(&SystemScope::mint(), WorkspaceId(Uuid::from_u128(0xA1)), id(emails[2]), false)).unwrap();
    make_workspace(&db, 0xA2, "guest-in", id(emails[3]), Role::Guest);
    make_workspace(&db, 0xA3, "member-in", id(emails[4]), Role::Member);
    make_workspace(&db, 0xA4, "admin-in", id(emails[5]), Role::Admin);

    let (who, slug) = match caller {
        Caller::Anonymous => ("", "owned"),
        Caller::Stranger => (emails[1], "owned"),
        Caller::Deactivated => (emails[2], "deactivated-in"),
        Caller::Guest => (emails[3], "guest-in"),
        Caller::Member => (emails[4], "member-in"),
        Caller::Admin => (emails[5], "admin-in"),
    };
    let r = call(&router, &jar, who, "GET", &row.path(slug), Value::Null);
    match expected(row, caller) {
        Some((status, body)) => assert_eq!((r["status"].clone(), r["body"].clone()), (json!(status), body), "{row:?} {caller:?}"),
        None => {
            assert_eq!(r["status"], 200, "{row:?} {caller:?}: {r}");
            assert!(row.shape_holds(&r["body"]), "{row:?} {caller:?}: {}", r["body"]);
        }
    }
    // a workspace that is not there is what a stranger gets
    if caller == Caller::Member {
        let r = call(&router, &jar, who, "GET", &row.path("no-such-workspace"), Value::Null);
        assert_eq!((r["status"].clone(), r["body"].clone()), (json!(403), row.refusal()), "{row:?} in a workspace that does not exist");
    }
}

// --- the project list -------------------------------------------------------------------------------------------------------

fn ts(t: Timestamp) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// A project as the pin's list names it. These are the 21 keys of the `.values()` call in `ProjectViewSet.list`; the stock app
/// reads each of them.
#[test]
fn a_project_is_listed_with_the_21_keys_of_the_pins_values_query() {
    let (router, db, jar) = world(&["admin@example.test", "lead@example.test"]);
    let (admin, lead) = (user_id(&db, "admin@example.test"), user_id(&db, "lead@example.test"));
    let ws = make_workspace(&db, 0xA, "listed", admin, Role::Admin);
    let project = ProjectId(Uuid::from_u128(0x1234));
    seed(
        &db,
        SeedFixture {
            projects: vec![SeedProject { id: project, workspace_id: ws, name: "Journey".into() }],
            project_details: vec![SeedProjectDetails {
                identifier: Some("JRN".into()),
                network: ProjectNetwork::Secret,
                archived: true,
                project_lead: Some(lead),
                created_by: Some(admin),
                created_at: Some(at(5)),
                ..SeedProjectDetails::of(project)
            }],
            memberships: vec![SeedMembership { project_id: project, workspace_id: ws, user_id: admin, role: Role::Member }],
            project_sort_orders: vec![SeedProjectSortOrder { project_id: project, user_id: admin, sort_order: 65535.0 }],
            ..SeedFixture::default()
        },
    );
    let r = call(&router, &jar, "admin@example.test", "GET", "/api/workspaces/listed/projects/", Value::Null);
    assert_eq!(r["status"], 200, "{r}");
    assert_eq!(
        r["body"],
        json!([{
            "id": project.0,
            "name": "Journey",
            "identifier": "JRN",
            "sort_order": 65535.0,
            "logo_props": {},
            "member_role": 15,
            "intake_count": 0,
            "archived_at": ts(at(5)),
            "workspace": ws.0,
            "cycle_view": false,
            "issue_views_view": false,
            "module_view": false,
            "page_view": true,
            "inbox_view": false,
            "guest_view_all_features": false,
            "project_lead": lead.0,
            "network": 0,
            "created_at": ts(at(5)),
            "updated_at": ts(at(5)),
            "created_by": admin.0,
            "updated_by": null,
        }])
    );
}

/// A project the caller has no row for says `null` for both their role and their sort order.
#[test]
fn a_project_the_caller_is_not_in_has_no_role_and_no_sort_order() {
    let (router, db, jar) = world(&["admin@example.test"]);
    let admin = user_id(&db, "admin@example.test");
    let ws = make_workspace(&db, 0xA, "listed", admin, Role::Admin);
    seed(&db, SeedFixture { projects: vec![SeedProject { id: ProjectId(Uuid::from_u128(1)), workspace_id: ws, name: "Not mine".into() }], ..SeedFixture::default() });
    let r = call(&router, &jar, "admin@example.test", "GET", "/api/workspaces/listed/projects/", Value::Null);
    let p = &r["body"][0];
    assert_eq!((p["member_role"].clone(), p["sort_order"].clone(), p["network"].clone()), (Value::Null, Value::Null, json!(2)), "{r}");
}

fn any_role() -> impl Strategy<Value = Role> {
    prop_oneof![Just(Role::Guest), Just(Role::Member), Just(Role::Admin)]
}

fn any_network() -> impl Strategy<Value = ProjectNetwork> {
    prop_oneof![Just(ProjectNetwork::Secret), Just(ProjectNetwork::Public)]
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// Whatever the caller's workspace role and however the projects are made, the route lists the projects that role can see,
    /// newest first, and names for each the caller's role number (null when they are not in it), the network number, and
    /// whether it is archived. The rule is the adapter's (`prop_the_list_is_the_visible_projects...` there); this holds the
    /// handler to carrying the caller's workspace role to it and the numbers to Plane's.
    #[test]
    fn prop_the_project_list_carries_the_workspace_role_and_plane_numbers(
        role in any_role(),
        projects in prop::collection::vec((any_network(), any::<bool>(), prop::option::of(any_role())), 0..8),
    ) {
        let (router, db, jar) = world(&["caller@example.test"]);
        let caller = user_id(&db, "caller@example.test");
        let ws = make_workspace(&db, 0xA, "carried", caller, role);
        let ids: Vec<ProjectId> = (0..projects.len()).map(|i| ProjectId(Uuid::from_u128(0x100 + i as u128))).collect();
        seed(&db, SeedFixture {
            projects: ids.iter().enumerate().map(|(i, id)| SeedProject { id: *id, workspace_id: ws, name: format!("p{i}") }).collect(),
            project_details: ids.iter().zip(&projects).enumerate().map(|(i, (id, (network, archived, _)))| SeedProjectDetails {
                network: *network,
                archived: *archived,
                // one second apart: a later project is a newer one
                created_at: Some(at(i as i64)),
                ..SeedProjectDetails::of(*id)
            }).collect(),
            memberships: ids.iter().zip(&projects).filter_map(|(id, (_, _, theirs))| {
                theirs.map(|r| SeedMembership { project_id: *id, workspace_id: ws, user_id: caller, role: r })
            }).collect(),
            ..SeedFixture::default()
        });
        let r = call(&router, &jar, "caller@example.test", "GET", "/api/workspaces/carried/projects/", Value::Null);
        prop_assert_eq!(&r["status"], &json!(200));

        // newest first: the last made is listed first
        let want: Vec<usize> = (0..projects.len()).rev().filter(|&i| {
            let (network, _, theirs) = &projects[i];
            match role {
                Role::Admin => true,
                Role::Member => theirs.is_some() || *network == ProjectNetwork::Public,
                Role::Guest => theirs.is_some(),
            }
        }).collect();
        let got = r["body"].as_array().unwrap();
        prop_assert_eq!(got.len(), want.len());
        for (g, i) in got.iter().zip(want) {
            let (network, archived, theirs) = &projects[i];
            prop_assert_eq!(&g["id"], &json!(ids[i].0));
            prop_assert_eq!(&g["member_role"], &json!(theirs.map(role_number)));
            prop_assert_eq!(&g["network"], &json!(if *network == ProjectNetwork::Secret { 0 } else { 2 }));
            prop_assert_eq!(g["archived_at"].is_null(), !*archived);
        }
    }
}

// --- the rows a first visit makes ---------------------------------------------------------------------------------------------

/// A member's rows are made once: the second visit is answered what the first was (the settings row keeps its id), a member
/// of the same workspace has rows of their own, and the same member in another workspace has others.
#[rstest]
fn a_second_visit_is_answered_what_the_first_was(#[values(Row::S21, Row::S22, Row::S24)] row: Row) {
    let (router, db, jar) = world(&["one@example.test", "two@example.test"]);
    let (one, two) = (user_id(&db, "one@example.test"), user_id(&db, "two@example.test"));
    let ws = make_workspace(&db, 0xA, "visited", one, Role::Admin);
    make_workspace(&db, 0xB, "elsewhere", one, Role::Admin);
    seed(&db, SeedFixture { users: vec![two], workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: two }], ..SeedFixture::default() });
    let ask = |who: &str, slug: &str| call(&router, &jar, who, "GET", &row.path(slug), Value::Null)["body"].clone();

    let first = ask("one@example.test", "visited");
    assert_eq!(ask("one@example.test", "visited"), first, "the second visit differs from the first");
    if row == Row::S22 {
        // the settings row has an id of its own, a creator and an owner, and they are the caller and the workspace
        assert_eq!((first["user"].clone(), first["created_by"].clone(), first["workspace"].clone()), (json!(one.0), json!(one.0), json!(ws.0)));
        assert_eq!((first["updated_by"].clone(), first["deleted_at"].clone()), (Value::Null, Value::Null));
        let other_member = ask("two@example.test", "visited");
        let other_workspace = ask("one@example.test", "elsewhere");
        assert_ne!(other_member["id"], first["id"], "two members share a row");
        assert_ne!(other_workspace["id"], first["id"], "two workspaces share a row");
        assert_eq!(other_member["user"], json!(two.0));
    } else {
        // the defaults are the same for everyone, and there are rows for each of them
        assert_eq!(ask("two@example.test", "visited"), first);
        assert_eq!(ask("one@example.test", "elsewhere"), first);
    }
}
