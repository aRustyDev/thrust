//! The states of Plane's v1 API (issue #58): list, create, retrieve, change and delete, through the crate's own router over the memory
//! adapters.
//!
//! What must hold, each pinned by the oracle's corpus (`states.json`) or read from the pin's source: the body's keys; a read is for any
//! member of the project and a write for a project ADMIN, so a member of the project is refused and so is a workspace admin who is not in
//! it; a create answers 200 and places the state after the greatest one that is not the intake state; the intake state is never shown;
//! DRF's field messages; the three answers a name that is taken gets; a change finds its state before it reads its body; and a delete
//! refuses the default state and a state that has work items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::BTreeSet;

use common::*;
use futures_executor::block_on;
use rstest::rstest;
use serde_json::{json, Value};
use thrust_contracts::*;
use thrust_core::*;

const DENIED: &str = "You do not have permission to perform this action.";
const NOT_FOUND: &str = "The requested resource does not exist.";
const PLANE_500: &str = "Something went wrong please try again later";

/// The keys of a state's body (`StateSerializer`).
const STATE_KEYS: [&str; 18] = [
    "color",
    "created_at",
    "created_by",
    "default",
    "deleted_at",
    "description",
    "external_id",
    "external_source",
    "group",
    "id",
    "is_triage",
    "name",
    "project",
    "sequence",
    "slug",
    "updated_at",
    "updated_by",
    "workspace",
];

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

fn set(of: &[&str]) -> BTreeSet<String> {
    of.iter().map(|s| (*s).to_string()).collect()
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn project_uri(project: u128) -> String {
    format!("/api/v1/workspaces/{SLUG}/projects/{}/", id(project).hyphenated())
}

fn states() -> String {
    format!("{}states/", project_uri(PROJECT))
}

fn state(n: impl std::fmt::Display) -> String {
    format!("{}{n}/", states())
}

fn get(w: &World, who: Who, uri: &str) -> Reply {
    send_as(&w.router, who.key(), "GET", uri)
}

fn call(w: &World, who: Who, method: &str, uri: &str, body: &Value) -> Reply {
    send_json(&w.router, who.key(), method, uri, body)
}

fn names(r: &Reply) -> Vec<String> {
    r.body["results"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect()
}

/// The ids by name of the project's states, from the list (the intake state is not in it).
fn by_name(w: &World, name: &str) -> String {
    get(w, Who::Admin, &states()).body["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("no state {name}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn defaults(w: &World) -> Vec<String> {
    get(w, Who::Admin, &states()).body["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["default"] == json!(true))
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect()
}

/// A work item of the cast project in `state`, made through core as a member would.
fn item_in(w: &World, state: &str) {
    let who = Principal { user_id: Who::Member.id() };
    let access = block_on(issue_writer(&w.app, &who, &WorkspaceSlug::new(SLUG), ProjectId(id(PROJECT)))).unwrap();
    let draft = IssueDraft { name: "an item".into(), state_id: Some(state.parse().unwrap()), ..IssueDraft::default() };
    block_on(create_issue(&w.app, &access, draft)).unwrap();
}

// --- the body, the list and a state ------------------------------------------------------------------------------------------

#[test]
fn a_state_has_the_pins_keys_and_values() {
    let w = real_world();
    let r = get(&w, Who::Guest, &state(id(STATE_TODO).hyphenated()));
    assert_eq!(r.status, 200);
    assert!(r.has_rate_headers());
    assert_eq!(keys(&r.body), set(&STATE_KEYS));
    let b = &r.body;
    assert_eq!(
        (b["name"].as_str(), b["slug"].as_str(), b["group"].as_str()),
        (Some("Todo"), Some("todo"), Some("unstarted"))
    );
    assert_eq!(
        (b["sequence"].clone(), b["default"].clone(), b["is_triage"].clone()),
        (json!(30000.0), json!(false), json!(false))
    );
    assert_eq!((b["description"].clone(), b["color"].clone()), (json!(""), json!("#000000")));
    assert_eq!(
        (b["external_source"].clone(), b["external_id"].clone(), b["deleted_at"].clone()),
        (Value::Null, Value::Null, Value::Null)
    );
    assert_eq!((b["created_by"].clone(), b["updated_by"].clone()), (Value::Null, Value::Null));
    assert_eq!((b["project"].clone(), b["workspace"].clone()), (json!(id(PROJECT)), json!(id(WS))));
    assert_eq!(b["id"], json!(id(STATE_TODO)));
}

/// The list is by `sequence`, leaves out the intake state, and has the pin's envelope; a guest reads it too.
#[test]
fn the_list_is_by_sequence_and_leaves_out_the_intake_state() {
    let w = real_world();
    for who in [Who::Admin, Who::Member, Who::Guest] {
        let r = get(&w, who, &states());
        assert_eq!(r.status, 200, "{who:?}");
        assert_eq!(names(&r), ["Backlog", "Todo"], "the intake state is not shown");
        assert_eq!(keys(&r.body["results"][0]), set(&STATE_KEYS));
        assert_eq!((r.body["total_results"].clone(), r.body["count"].clone()), (json!(2), json!(2)));
        assert_eq!(r.body["next_cursor"], json!("1000:1:0"));
    }
    let outsider = get(&w, Who::Outsider, &states());
    assert_eq!((outsider.status, outsider.body), (403, json!({ "detail": DENIED })));
}

#[test]
fn fields_and_pages_work_on_the_list_as_on_projects() {
    let w = real_world();
    let r = get(&w, Who::Admin, &format!("{}?fields=id,name", states()));
    assert_eq!(keys(&r.body["results"][0]), set(&["id", "name"]));
    // two pages of one
    let first = get(&w, Who::Admin, &format!("{}?per_page=1", states()));
    assert_eq!(
        (names(&first), first.body["next_page_results"].clone(), first.body["total_pages"].clone()),
        (vec!["Backlog".to_string()], json!(true), json!(2))
    );
    let second =
        get(&w, Who::Admin, &format!("{}?per_page=1&cursor={}", states(), first.body["next_cursor"].as_str().unwrap()));
    assert_eq!((names(&second), second.body["next_page_results"].clone()), (vec!["Todo".to_string()], json!(false)));
    // what the corpus records for a list asked wrongly is counted, and so carries the rate headers
    for (q, status) in [("per_page=abc", 400), ("per_page=1001", 400), ("cursor=garbage", 400), ("per_page=0", 500)] {
        let r = get(&w, Who::Admin, &format!("{}?{q}", states()));
        assert_eq!(r.status, status, "{q}");
        assert!(r.has_rate_headers(), "{q}");
    }
}

#[test]
fn a_state_that_is_not_shown_is_not_found() {
    let w = real_world();
    // the intake state, one that is nowhere, and one that is not a uuid
    for (uri, body) in [
        (state(id(STATE_TRIAGE).hyphenated()), json!({ "error": NOT_FOUND })),
        (state(id(0xBAD).hyphenated()), json!({ "error": NOT_FOUND })),
        (state("nope"), json!({ "error": "Page not found." })),
    ] {
        let r = get(&w, Who::Admin, &uri);
        assert_eq!((r.status, r.body), (404, body), "{uri}");
    }
    // a project that is not there is a 403, as for every route that asks for membership
    let r = get(&w, Who::Admin, &format!("{}states/", project_uri(0xBAD)));
    assert_eq!((r.status, r.body), (403, json!({ "detail": DENIED })));
}

/// The pin's reads leave out the states of an archived project (the list is empty, a state is not found); its writes do not look.
#[test]
fn an_archived_project_shows_no_states_but_its_states_can_still_be_changed() {
    let w = real_world();
    let todo = id(STATE_TODO).hyphenated().to_string();
    assert_eq!(call(&w, Who::Admin, "POST", &format!("{}archive/", project_uri(PROJECT)), &json!({})).status, 204);
    let list = get(&w, Who::Admin, &states());
    assert_eq!((list.status, names(&list), list.body["total_results"].clone()), (200, Vec::<String>::new(), json!(0)));
    assert_eq!(get(&w, Who::Admin, &state(&todo)).status, 404);
    let r = call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"name": "Still here"}));
    assert_eq!((r.status, r.body["name"].clone()), (200, json!("Still here")));
    assert_eq!(call(&w, Who::Admin, "DELETE", &state(&todo), &json!({})).status, 204);
}

// --- who may write ------------------------------------------------------------------------------------------------------------

/// A write is for a project admin: a member of the project is refused, a guest is, so is a stranger, and so is an admin of the
/// WORKSPACE who is not in the project (here, one the member made). A refusal is not counted. The control: the same writes by the
/// project's admin succeed.
#[test]
fn a_write_is_for_a_project_admin_and_no_one_else() {
    let write = |w: &World, who: Who, project: u128| -> Vec<u16> {
        let base = format!("{}states/", project_uri(project));
        let created = call(w, who, "POST", &base, &json!({"name": format!("by {who:?}"), "color": "#fff"}));
        // the other two writes name a state that exists (or would): refused before it matters
        let other = format!("{base}{}/", id(STATE_TODO).hyphenated());
        let patched = call(w, who, "PATCH", &other, &json!({"name": format!("renamed by {who:?}")}));
        let deleted = call(w, who, "DELETE", &other, &json!({}));
        vec![created.status, patched.status, deleted.status]
    };
    // the cast project: its admin writes, and no one else does
    for (who, want) in [
        (Who::Admin, vec![200, 200, 204]),
        (Who::Member, vec![403, 403, 403]),
        (Who::Guest, vec![403, 403, 403]),
        (Who::Outsider, vec![403, 403, 403]),
        (Who::Inactive, vec![403, 403, 403]),
    ] {
        let w = real_world();
        assert_eq!(write(&w, who, PROJECT), want, "{who:?} on the cast project");
    }
    // a project the member made: the member is its admin, and the workspace admin is not in it, so is refused
    let w = real_world();
    let made = block_on(create_project(
        &w.app,
        &block_on(authorize_workspace(&w.app, &Principal { user_id: Who::Member.id() }, &WorkspaceSlug::new(SLUG)))
            .unwrap(),
        ProjectPatch { name: Some("Members".into()), identifier: Some("MEM".into()), ..ProjectPatch::default() },
    ))
    .unwrap();
    let project = made.listing.project.id.0.as_u128();
    let state_base = format!("{}states/", project_uri(project));
    let admin = call(&w, Who::Admin, "POST", &state_base, &json!({"name": "Nope", "color": "#fff"}));
    assert_eq!(
        (admin.status, admin.body.clone()),
        (403, json!({ "detail": DENIED })),
        "a workspace admin is not a project admin"
    );
    assert!(!admin.has_rate_headers(), "a refused request is not counted");
    assert_eq!(call(&w, Who::Member, "POST", &state_base, &json!({"name": "Yes", "color": "#fff"})).status, 200);
    // and a project that is not there is refused to everyone
    let nowhere = format!("{}states/", project_uri(0xBAD));
    for who in [Who::Admin, Who::Member] {
        assert_eq!(call(&w, who, "POST", &nowhere, &json!({"name": "x", "color": "#fff"})).status, 403, "{who:?}");
    }
}

// --- making a state -----------------------------------------------------------------------------------------------------------

/// The corpus's creation (`create`): 200, not 201; the place is the greatest that is not the intake state, plus 15000 (30000 here;
/// the intake state at 90000 does not count); the slug follows the name; the maker is the admin and nobody has changed it yet.
#[test]
fn a_new_state_answers_200_and_goes_after_the_greatest_one_that_is_not_the_intake_state() {
    let w = real_world();
    let r = call(
        &w,
        Who::Admin,
        "POST",
        &states(),
        &json!({"name": "Oracle Review", "color": "#123456", "group": "started", "sequence": 1}),
    );
    assert_eq!(r.status, 200);
    assert!(r.has_rate_headers());
    assert_eq!(keys(&r.body), set(&STATE_KEYS));
    let b = &r.body;
    assert_eq!(
        (b["sequence"].clone(), b["slug"].clone()),
        (json!(45000.0), json!("oracle-review")),
        "30000 + 15000, not 90000 + 15000"
    );
    assert_eq!(
        (b["group"].clone(), b["default"].clone(), b["is_triage"].clone()),
        (json!("started"), json!(false), json!(false))
    );
    assert_eq!((b["created_by"].clone(), b["updated_by"].clone()), (json!(Who::Admin.id().0), Value::Null));
    assert_eq!((b["description"].clone(), b["color"].clone()), (json!(""), json!("#123456")));
    assert_eq!((b["project"].clone(), b["workspace"].clone()), (json!(id(PROJECT)), json!(id(WS))));
    // the next goes after that one, and the list shows both in place
    let r2 = call(&w, Who::Admin, "POST", &states(), &json!({"name": "QA", "color": "#fff"}));
    assert_eq!(r2.body["sequence"], json!(60000.0));
    assert_eq!(names(&get(&w, Who::Admin, &states())), ["Backlog", "Todo", "Oracle Review", "QA"]);
    // the group is `backlog` when none is named
    assert_eq!(r2.body["group"], json!("backlog"));
}

/// DRF's messages for a creation. The two first come from the corpus (`create-missing-color`, `create-invalid-group`).
#[rstest]
#[case::no_color(json!({"name": "No Colour"}), json!({"color": ["This field is required."]}))]
#[case::invalid_group(json!({"name": "Bad Group", "color": "#123456", "group": "nope"}), json!({"group": ["\"nope\" is not a valid choice."]}))]
#[case::nothing(json!({}), json!({"name": ["This field is required."], "color": ["This field is required."]}))]
#[case::blank_name(json!({"name": "  ", "color": "#fff"}), json!({"name": ["This field may not be blank."]}))]
#[case::null_name(json!({"name": null, "color": "#fff"}), json!({"name": ["This field may not be null."]}))]
#[case::a_long_name(json!({"name": "n".repeat(256), "color": "#fff"}), json!({"name": ["Ensure this field has no more than 255 characters."]}))]
#[case::a_long_colour(json!({"name": "n", "color": "c".repeat(256)}), json!({"color": ["Ensure this field has no more than 255 characters."]}))]
#[case::a_null_description(json!({"name": "n", "color": "#fff", "description": null}), json!({"description": ["This field may not be null."]}))]
#[case::not_a_number(json!({"name": "n", "color": "#fff", "sequence": "x"}), json!({"sequence": ["A valid number is required."]}))]
#[case::not_a_boolean(json!({"name": "n", "color": "#fff", "default": "maybe"}), json!({"default": ["Must be a valid boolean."]}))]
#[case::is_triage_not_a_boolean(json!({"name": "n", "color": "#fff", "is_triage": "x"}), json!({"is_triage": ["Must be a valid boolean."]}))]
#[case::another_case_of_a_group(json!({"name": "n", "color": "#fff", "group": "Started"}), json!({"group": ["\"Started\" is not a valid choice."]}))]
#[case::a_null_group(json!({"name": "n", "color": "#fff", "group": null}), json!({"group": ["This field may not be null."]}))]
#[case::a_null_colour(json!({"name": "n", "color": null}), json!({"color": ["This field may not be null."]}))]
#[case::several(json!({"group": "x", "sequence": "y"}), json!({"name": ["This field is required."], "color": ["This field is required."], "group": ["\"x\" is not a valid choice."], "sequence": ["A valid number is required."]}))]
fn a_creation_with_a_field_wrong_is_refused_with_drfs_message(#[case] body: Value, #[case] errors: Value) {
    let w = real_world();
    let r = call(&w, Who::Admin, "POST", &states(), &body);
    assert_eq!((r.status, &r.body), (400, &errors));
    assert!(r.has_rate_headers());
    assert_eq!(names(&get(&w, Who::Admin, &states())), ["Backlog", "Todo"], "nothing was made");
}

/// `validate()` refuses the group `triage` (it is the intake's), after every field was fine: a field error hides it.
#[test]
fn a_triage_state_is_not_made_by_v1_and_only_when_every_field_was_fine() {
    let w = real_world();
    let r = call(&w, Who::Admin, "POST", &states(), &json!({"name": "Intake", "color": "#fff", "group": "triage"}));
    assert_eq!((r.status, r.body), (400, json!({"non_field_errors": ["Cannot create triage state"]})));
    let hidden = call(&w, Who::Admin, "POST", &states(), &json!({"name": "Intake", "group": "triage"}));
    assert_eq!(hidden.body, json!({"color": ["This field is required."]}), "a field error comes before validate()");
}

/// The three answers to a name that is taken: a creation is a 409 naming the state that has it (the corpus's
/// `create-duplicate-name`), a clash with the intake state, which the pin does not show, is the pin's accidental failure (its lookup
/// finds nothing and it raises: a 500), and a change is the base view's 400.
#[test]
fn a_name_that_is_taken_gets_three_different_answers() {
    let w = real_world();
    let todo = id(STATE_TODO).hyphenated().to_string();
    let r = call(&w, Who::Admin, "POST", &states(), &json!({"name": "Todo", "color": "#fff"}));
    assert_eq!(
        (r.status, r.body),
        (409, json!({"error": "State with the same name already exists in the project", "id": todo}))
    );
    // the pin looks the clash up by the name as it was sent, which DRF had trimmed: spaces round a taken name find nothing to name
    let padded = call(&w, Who::Admin, "POST", &states(), &json!({"name": " Todo ", "color": "#fff"}));
    assert_eq!((padded.status, padded.body.clone()), (500, json!({"error": PLANE_500})));
    assert!(padded.has_rate_headers());
    // spaces round a name that is free are trimmed away, and the state has the trimmed name
    let made = call(&w, Who::Admin, "POST", &states(), &json!({"name": "  Padded  ", "color": "#fff"}));
    assert_eq!((made.status, made.body["name"].clone()), (200, json!("Padded")));
    // another case is another name, and a deleted state's name is free
    assert_eq!(call(&w, Who::Admin, "POST", &states(), &json!({"name": "todo", "color": "#fff"})).status, 200);
    let r = call(&w, Who::Admin, "POST", &states(), &json!({"name": "Triage", "color": "#fff"}));
    assert_eq!(
        (r.status, r.body.clone()),
        (500, json!({"error": PLANE_500})),
        "a clash with a state that is not shown"
    );
    assert!(r.has_rate_headers());
    // a change onto a name another state has
    let r = call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"name": "Backlog"}));
    assert_eq!((r.status, r.body), (400, json!({"error": "The payload is not valid"})));
}

/// A key (`external_source` and `external_id`) that another state has is refused only when both halves are named, and the answer
/// names the state that has it.
#[test]
fn an_integration_key_that_another_state_has_is_a_409_naming_it() {
    let w = real_world();
    let first = call(
        &w,
        Who::Admin,
        "POST",
        &states(),
        &json!({"name": "One", "color": "#fff", "external_source": "gh", "external_id": "1"}),
    );
    assert_eq!((first.body["external_source"].clone(), first.body["external_id"].clone()), (json!("gh"), json!("1")));
    let taken = call(
        &w,
        Who::Admin,
        "POST",
        &states(),
        &json!({"name": "Two", "color": "#fff", "external_source": "gh", "external_id": "1"}),
    );
    assert_eq!(
        (taken.status, taken.body),
        (
            409,
            json!({"error": "State with the same external id and external source already exists", "id": first.body["id"]})
        )
    );
    for body in [
        json!({"name": "Three", "color": "#fff", "external_source": "jira", "external_id": "1"}),
        json!({"name": "Four", "color": "#fff", "external_source": "gh", "external_id": "2"}),
        json!({"name": "Five", "color": "#fff", "external_id": "1"}),
        json!({"name": "Six", "color": "#fff", "external_source": "gh"}),
    ] {
        assert_eq!(call(&w, Who::Admin, "POST", &states(), &body).status, 200, "{body}");
    }
}

/// A body that is not an object: a list is DRF's "Expected a dictionary" 400; the other cases are the same as for projects.
#[test]
fn a_body_that_is_not_an_object_is_refused() {
    let w = real_world();
    let post = |ct: &str, body: &str| {
        send_raw(
            &w.router,
            "POST",
            &states(),
            &[("x-api-key", Who::Admin.key()), ("content-type", ct)],
            body.as_bytes().to_vec(),
        )
    };
    let r = post("application/json", "[1]");
    assert_eq!(
        (r.status, r.body),
        (400, json!({"non_field_errors": ["Invalid data. Expected a dictionary, but got list."]}))
    );
    assert_eq!(post("text/plain", "name=x").status, 415);
    assert_eq!(post("application/json", "{bad").status, 400);
    assert_eq!(
        post("application/json", "\"s\"").body,
        json!({"detail": "Request body must be a JSON object or array."})
    );
    let empty = send_raw(&w.router, "POST", &states(), &[("x-api-key", Who::Admin.key())], Vec::new());
    assert_eq!(empty.body, json!({"name": ["This field is required."], "color": ["This field is required."]}));
}

// --- the default --------------------------------------------------------------------------------------------------------------

#[test]
fn a_state_that_is_made_or_changed_the_default_takes_it_from_the_others() {
    let w = real_world();
    assert_eq!(defaults(&w), ["Backlog"]);
    let r = call(&w, Who::Admin, "POST", &states(), &json!({"name": "Review", "color": "#fff", "default": true}));
    assert_eq!((r.status, r.body["default"].clone()), (200, json!(true)));
    assert_eq!(defaults(&w), ["Review"]);
    let todo = id(STATE_TODO).hyphenated().to_string();
    assert_eq!(call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"default": true})).body["default"], json!(true));
    assert_eq!(defaults(&w), ["Todo"]);
    // a state that is the default and is changed to be none leaves the project with none
    call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"default": false}));
    assert!(defaults(&w).is_empty());
}

// --- changing a state ---------------------------------------------------------------------------------------------------------

/// The corpus's change (`patch`): the slug follows the new name, the place is kept, and who changed it is recorded.
#[test]
fn a_change_sets_what_it_names_and_records_who_changed_it() {
    let w = real_world();
    let todo = id(STATE_TODO).hyphenated().to_string();
    let r = call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"name": "Oracle Review 2"}));
    assert_eq!(r.status, 200);
    assert!(r.has_rate_headers());
    assert_eq!(keys(&r.body), set(&STATE_KEYS));
    assert_eq!((r.body["name"].clone(), r.body["slug"].clone()), (json!("Oracle Review 2"), json!("oracle-review-2")));
    assert_eq!((r.body["sequence"].clone(), r.body["group"].clone()), (json!(30000.0), json!("unstarted")), "kept");
    assert_eq!(r.body["updated_by"], json!(Who::Admin.id().0));
    assert_eq!(r.body["created_by"], Value::Null, "who made it is kept");
    // a change can move its place and its group, set a description, a colour and a key, and clear a key
    let r = call(
        &w,
        Who::Admin,
        "PATCH",
        &state(&todo),
        &json!({"sequence": "123.5", "group": "completed", "description": "d", "color": "#abc", "external_source": "gh", "external_id": "9"}),
    );
    assert_eq!(
        (r.body["sequence"].clone(), r.body["group"].clone(), r.body["description"].clone(), r.body["color"].clone()),
        (json!(123.5), json!("completed"), json!("d"), json!("#abc"))
    );
    assert_eq!((r.body["external_source"].clone(), r.body["external_id"].clone()), (json!("gh"), json!("9")));
    let r = call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"external_source": null, "external_id": null}));
    assert_eq!((r.body["external_source"].clone(), r.body["external_id"].clone()), (Value::Null, Value::Null));
    // naming nothing is a change that changes nothing but who and when
    assert_eq!(call(&w, Who::Admin, "PATCH", &state(&todo), &json!({})).status, 200);
}

/// The base view parses the body before the handler runs (`BaseAPIView.initial` reads `request.data`), so a body that cannot be parsed
/// is refused whether or not the state is there; the handler then finds the state, and only then does the serializer read what the
/// parsed body says: a list, or a field that is wrong, comes after the 404. The intake state is not found.
#[test]
fn a_change_parses_the_body_then_finds_its_state_then_reads_what_it_says() {
    let w = real_world();
    let todo = id(STATE_TODO).hyphenated().to_string();
    let nowhere = state(id(0xBAD).hyphenated());
    let patch = |uri: &str, ct: &str, body: &str| {
        send_raw(
            &w.router,
            "PATCH",
            uri,
            &[("x-api-key", Who::Admin.key()), ("content-type", ct)],
            body.as_bytes().to_vec(),
        )
    };
    // parse-level refusals come first, for a state that is not there as for one that is
    for uri in [&nowhere, &state(&todo)] {
        assert_eq!(patch(uri, "text/plain", "x").status, 415, "{uri}");
        let bad = patch(uri, "application/json", "{bad");
        assert_eq!(bad.status, 400, "{uri}");
        assert!(bad.body["detail"].as_str().unwrap().starts_with("JSON parse error - "), "{uri}");
        let scalar = patch(uri, "application/json", "\"s\"");
        assert_eq!(scalar.body, json!({"detail": "Request body must be a JSON object or array."}), "{uri}");
    }
    // a list parses, so the state is looked for before the serializer refuses it
    let r = patch(&nowhere, "application/json", "[1]");
    assert_eq!((r.status, r.body), (404, json!({ "error": NOT_FOUND })));
    let r = patch(&state(&todo), "application/json", "[1]");
    assert_eq!(r.body, json!({"non_field_errors": ["Invalid data. Expected a dictionary, but got list."]}));
    // so is a field that is wrong, and an empty body is no data at all
    let r = patch(&nowhere, "application/json", "{\"group\": \"nope\"}");
    assert_eq!((r.status, r.body), (404, json!({ "error": NOT_FOUND })));
    assert_eq!(patch(&nowhere, "text/plain", "").status, 404, "an empty body is read as no data, whatever its type");
    // the intake state is not found, and a field that is wrong on a state that is there is a 400
    let intake = state(id(STATE_TRIAGE).hyphenated());
    assert_eq!(call(&w, Who::Admin, "PATCH", &intake, &json!({"name": "x"})).status, 404);
    let r = call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"group": "nope"}));
    assert_eq!((r.status, r.body), (400, json!({"group": ["\"nope\" is not a valid choice."]})));
}

/// The group `triage` is refused on a change as on a creation (corpus: `patch-invalid-group` is the choice error; this is
/// `validate()`), and `is_triage` does not make a state the intake's.
#[test]
fn a_change_cannot_make_a_state_a_triage_state() {
    let w = real_world();
    let todo = id(STATE_TODO).hyphenated().to_string();
    let r = call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"group": "triage"}));
    assert_eq!((r.status, r.body), (400, json!({"non_field_errors": ["Cannot create triage state"]})));
    let r = call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"is_triage": true}));
    assert_eq!((r.status, r.body["is_triage"].clone()), (200, json!(false)), "the group says it, here");
    assert_eq!(get(&w, Who::Admin, &state(&todo)).status, 200, "and it is still shown");
}

/// A key that is new is refused when another state has it, and the answer names the state being changed (not the one that has it).
#[test]
fn a_change_to_a_key_that_another_state_has_names_the_state_being_changed() {
    let w = real_world();
    let todo = id(STATE_TODO).hyphenated().to_string();
    call(
        &w,
        Who::Admin,
        "POST",
        &states(),
        &json!({"name": "Keyed", "color": "#fff", "external_source": "gh", "external_id": "1"}),
    );
    let r = call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"external_source": "gh", "external_id": "1"}));
    assert_eq!(
        (r.status, r.body),
        (409, json!({"error": "State with the same external id and external source already exists", "id": todo}))
    );
    // another source is another key, and the state that has the key keeps it
    assert_eq!(
        call(&w, Who::Admin, "PATCH", &state(&todo), &json!({"external_source": "jira", "external_id": "1"})).status,
        200
    );
    let keyed = by_name(&w, "Keyed");
    assert_eq!(
        call(&w, Who::Admin, "PATCH", &state(&keyed), &json!({"external_source": "gh", "external_id": "1"})).status,
        200
    );
}

// --- deleting a state ---------------------------------------------------------------------------------------------------------

#[test]
fn a_state_is_deleted_unless_it_is_the_default_or_has_work_items() {
    let w = real_world();
    let backlog = id(STATE_BACKLOG).hyphenated().to_string();
    let todo = id(STATE_TODO).hyphenated().to_string();
    // the default (the corpus's `delete-default-state`)
    let r = call(&w, Who::Admin, "DELETE", &state(&backlog), &json!({}));
    assert_eq!((r.status, r.body.clone()), (400, json!({"error": "Default state cannot be deleted"})));
    assert!(r.has_rate_headers());
    // one with a work item in it
    item_in(&w, &todo);
    let r = call(&w, Who::Admin, "DELETE", &state(&todo), &json!({}));
    assert_eq!((r.status, r.body), (400, json!({"error": "The state is not empty, only empty states can be deleted"})));
    // one that is empty (the corpus's `delete`): 204 with no body, and then it is gone
    let made = call(&w, Who::Admin, "POST", &states(), &json!({"name": "Empty", "color": "#fff"}));
    let empty = made.body["id"].as_str().unwrap().to_string();
    let r = call(&w, Who::Admin, "DELETE", &state(&empty), &json!({}));
    assert_eq!((r.status, r.body.clone()), (204, Value::Null));
    assert!(r.has_rate_headers());
    let gone = get(&w, Who::Admin, &state(&empty));
    assert_eq!((gone.status, gone.body), (404, json!({ "error": NOT_FOUND })), "the corpus's `detail-after-delete`");
    let again = call(&w, Who::Admin, "DELETE", &state(&empty), &json!({}));
    assert_eq!((again.status, again.body), (404, json!({ "error": NOT_FOUND })));
    assert_eq!(names(&get(&w, Who::Admin, &states())), ["Backlog", "Todo"]);
    // its name is free
    assert_eq!(call(&w, Who::Admin, "POST", &states(), &json!({"name": "Empty", "color": "#fff"})).status, 200);
    // the intake state and one that is nowhere are not there to delete
    for hidden in [id(STATE_TRIAGE), id(0xBAD)] {
        assert_eq!(call(&w, Who::Admin, "DELETE", &state(hidden.hyphenated()), &json!({})).status, 404, "{hidden}");
    }
}
