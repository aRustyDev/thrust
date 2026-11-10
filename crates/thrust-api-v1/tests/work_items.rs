//! The work items of Plane's v1 API: list, create, retrieve, change and delete, the search and the lookup by `KEY-n`, in both families
//! of paths, through the crate's own router over the memory adapters.
//!
//! What must hold, each pinned by the oracle's corpus (`work_items.json`) or read from the pin's source: the 29-key body; any member of
//! the project reads every work item (a guest too) and an admin or a member writes; a creation takes the project's default state,
//! places the item after the greatest in its state and answers 201; DRF's field messages and `validate()`'s refusals; a key that
//! another work item has is a 409; a body that cannot be parsed is refused before the work item is looked for; a deletion is for a
//! project admin or the maker and takes what is under it; the list's refusals, order and single-object mode; and the search and the
//! lookup by identifier.
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
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Value};
use thrust_contracts::*;
use thrust_core::*;

const DENIED: &str = "You do not have permission to perform this action.";
const NOT_FOUND: &str = "The requested resource does not exist.";
const PLANE_500: &str = "Something went wrong please try again later";

/// The keys of a work item's body (`IssueSerializer`).
const KEYS: [&str; 29] = [
    "archived_at",
    "assignees",
    "completed_at",
    "created_at",
    "created_by",
    "deleted_at",
    "description_binary",
    "description_html",
    "estimate_point",
    "external_id",
    "external_source",
    "id",
    "is_draft",
    "labels",
    "name",
    "parent",
    "point",
    "priority",
    "project",
    "sequence_id",
    "sort_order",
    "start_date",
    "state",
    "target_date",
    "type",
    "type_id",
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

/// The collection of the cast project's work items, in a family of paths.
fn items(family: &str) -> String {
    format!("{}{family}/", project_uri(PROJECT))
}

fn item(family: &str, n: impl std::fmt::Display) -> String {
    format!("{}{n}/", items(family))
}

fn get(w: &World, who: Who, uri: &str) -> Reply {
    send_as(&w.router, who.key(), "GET", uri)
}

fn call(w: &World, who: Who, method: &str, uri: &str, body: &Value) -> Reply {
    send_json(&w.router, who.key(), method, uri, body)
}

fn raw(w: &World, who: Who, method: &str, uri: &str, content_type: &str, body: &str) -> Reply {
    send_raw(
        &w.router,
        method,
        uri,
        &[("x-api-key", who.key()), ("content-type", content_type)],
        body.as_bytes().to_vec(),
    )
}

/// A work item made through the API by `who`; its body.
fn make(w: &World, who: Who, body: Value) -> Value {
    let r = call(w, who, "POST", &items("work-items"), &body);
    assert_eq!(r.status, 201, "{}", r.body);
    r.body
}

fn id_of(body: &Value) -> String {
    body["id"].as_str().unwrap().to_string()
}

fn names(r: &Reply) -> Vec<String> {
    r.body["results"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect()
}

fn listed(w: &World) -> Vec<String> {
    names(&get(w, Who::Admin, &items("work-items")))
}

fn backlog() -> Value {
    json!(id(STATE_BACKLOG))
}

/// A project that `who` makes through core, so they are its admin and the others are not in it.
fn project_of(w: &World, who: Who, identifier: &str) -> u128 {
    let principal = Principal { user_id: who.id() };
    let scope = block_on(authorize_workspace(&w.app, &principal, &WorkspaceSlug::new(SLUG))).unwrap();
    let patch = ProjectPatch {
        name: Some(format!("Other {identifier}")),
        identifier: Some(identifier.into()),
        ..ProjectPatch::default()
    };
    block_on(create_project(&w.app, &scope, patch)).unwrap().listing.project.id.0.as_u128()
}

fn archive(w: &World) {
    assert_eq!(call(w, Who::Admin, "POST", &format!("{}archive/", project_uri(PROJECT)), &json!({})).status, 204);
}

/// The identifier of the cast project, as the API says it.
fn identifier(w: &World) -> String {
    get(w, Who::Admin, &project_uri(PROJECT)).body["identifier"].as_str().unwrap().to_string()
}

// --- the body, a creation ------------------------------------------------------------------------------------------------------

/// The corpus's `create-minimal`: 201; the default state; the first place in it; the caller made it and nobody has changed it; an
/// empty paragraph for a description. Both families answer the same.
#[rstest]
fn a_new_work_item_answers_201_with_the_pins_keys_and_defaults(#[values("issues", "work-items")] family: &str) {
    let w = real_world();
    let r = call(&w, Who::Admin, "POST", &items(family), &json!({"name": "Scratch item A"}));
    assert_eq!(r.status, 201);
    assert!(r.has_rate_headers());
    assert_eq!(keys(&r.body), set(&KEYS));
    let b = &r.body;
    assert_eq!(
        (b["name"].clone(), b["priority"].clone(), b["description_html"].clone()),
        (json!("Scratch item A"), json!("none"), json!("<p></p>"))
    );
    assert_eq!(
        (b["state"].clone(), b["sequence_id"].clone(), b["sort_order"].clone()),
        (backlog(), json!(1), json!(65535.0))
    );
    assert_eq!((b["created_by"].clone(), b["updated_by"].clone()), (json!(Who::Admin.id().0), Value::Null));
    for null in [
        "parent",
        "point",
        "start_date",
        "target_date",
        "completed_at",
        "archived_at",
        "deleted_at",
        "estimate_point",
        "type",
        "type_id",
        "external_id",
        "external_source",
        "description_binary",
    ] {
        assert_eq!(b[null], Value::Null, "{null}");
    }
    assert_eq!(
        (b["is_draft"].clone(), b["assignees"].clone(), b["labels"].clone()),
        (json!(false), json!([]), json!([]))
    );
    assert_eq!((b["project"].clone(), b["workspace"].clone()), (json!(id(PROJECT)), json!(id(WS))));
    // a second one is numbered after it and placed 10000 after it in the same state
    let second = make(&w, Who::Member, json!({"name": "second"}));
    assert_eq!((second["sequence_id"].clone(), second["sort_order"].clone()), (json!(2), json!(75535.0)));
    assert_eq!(second["created_by"], json!(Who::Member.id().0));
}

/// The corpus's `create-full`: the people, the labels, the priority, the state and a description are kept.
#[test]
fn a_creation_keeps_what_it_names() {
    let w = real_world();
    let todo = id(STATE_TODO);
    let b = make(
        &w,
        Who::Admin,
        json!({
            "name": "Scratch item B", "priority": "high", "state": todo, "description_html": "<p>scratch</p>",
            "assignees": [Who::Member.id().0], "labels": [id(LABEL_BUG)],
            "start_date": "2026-11-01", "target_date": "2026-11-30", "point": 5,
            "external_source": "gh", "external_id": "7",
        }),
    );
    assert_eq!(
        (b["priority"].clone(), b["state"].clone(), b["description_html"].clone()),
        (json!("high"), json!(todo), json!("<p>scratch</p>"))
    );
    assert_eq!((b["assignees"].clone(), b["labels"].clone()), (json!([Who::Member.id().0]), json!([id(LABEL_BUG)])));
    assert_eq!(
        (b["start_date"].clone(), b["target_date"].clone(), b["point"].clone()),
        (json!("2026-11-01"), json!("2026-11-30"), json!(5))
    );
    assert_eq!((b["external_source"].clone(), b["external_id"].clone()), (json!("gh"), json!("7")));
    // what is stored is what a read finds
    let read = get(&w, Who::Guest, &item("work-items", id_of(&b)));
    assert_eq!(read.body, b);
    // a work item in a state of the completed group is completed
    let done =
        call(&w, Who::Admin, "POST", &states_uri(), &json!({"name": "Done", "color": "#0f0", "group": "completed"}));
    let completed = make(&w, Who::Admin, json!({"name": "done", "state": done.body["id"]}));
    assert_eq!(completed["state"], done.body["id"]);
    assert!(completed["completed_at"].is_string(), "{}", completed["completed_at"]);
}

fn states_uri() -> String {
    format!("{}states/", project_uri(PROJECT))
}

/// DRF's messages for a creation. The first four come from the corpus (`create-missing-name`, `create-invalid-priority`,
/// `create-unknown-state`).
#[rstest]
#[case::no_name(json!({"priority": "low"}), json!({"name": ["This field is required."]}))]
#[case::blank_name(json!({"name": "  "}), json!({"name": ["This field may not be blank."]}))]
#[case::null_name(json!({"name": null}), json!({"name": ["This field may not be null."]}))]
#[case::a_long_name(json!({"name": "n".repeat(256)}), json!({"name": ["Ensure this field has no more than 255 characters."]}))]
#[case::bad_priority(json!({"name": "x", "priority": "super"}), json!({"priority": ["\"super\" is not a valid choice."]}))]
#[case::unknown_state(json!({"name": "x", "state": "5a1b3c4d-0000-4000-8000-000000000001"}), json!({"state": ["Invalid pk \"5a1b3c4d-0000-4000-8000-000000000001\" - object does not exist."]}))]
#[case::the_intake_state(json!({"name": "x", "state": id(STATE_TRIAGE)}), json!({"state": [format!("Invalid pk \"{}\" - object does not exist.", id(STATE_TRIAGE))]}))]
#[case::a_state_that_is_a_bool(json!({"name": "x", "state": true}), json!({"state": ["Incorrect type. Expected pk value, received bool."]}))]
#[case::unknown_parent(json!({"name": "x", "parent": "5a1b3c4d-0000-4000-8000-000000000001"}), json!({"parent": ["Invalid pk \"5a1b3c4d-0000-4000-8000-000000000001\" - object does not exist."]}))]
#[case::unknown_label(json!({"name": "x", "labels": ["5a1b3c4d-0000-4000-8000-000000000001"]}), json!({"labels": {"0": ["Invalid pk \"5a1b3c4d-0000-4000-8000-000000000001\" - object does not exist."]}}))]
#[case::labels_not_a_list(json!({"name": "x", "labels": "bug"}), json!({"labels": ["Expected a list of items but got type \"str\"."]}))]
#[case::labels_null(json!({"name": "x", "labels": null}), json!({"labels": ["This field may not be null."]}))]
#[case::a_null_label(json!({"name": "x", "labels": [null]}), json!({"labels": {"0": ["This field may not be null."]}}))]
#[case::unknown_assignee(json!({"name": "x", "assignees": ["5a1b3c4d-0000-4000-8000-000000000001"]}), json!({"assignees": {"0": ["Invalid pk \"5a1b3c4d-0000-4000-8000-000000000001\" - object does not exist."]}}))]
#[case::a_type(json!({"name": "x", "type_id": "5a1b3c4d-0000-4000-8000-000000000001"}), json!({"type_id": ["Invalid pk \"5a1b3c4d-0000-4000-8000-000000000001\" - object does not exist."]}))]
#[case::a_type_by_its_model_name(json!({"name": "x", "type": "5a1b3c4d-0000-4000-8000-000000000001"}), json!({"type": ["Invalid pk \"5a1b3c4d-0000-4000-8000-000000000001\" - object does not exist."]}))]
#[case::an_estimate(json!({"name": "x", "estimate_point": "5a1b3c4d-0000-4000-8000-000000000001"}), json!({"estimate_point": ["Invalid pk \"5a1b3c4d-0000-4000-8000-000000000001\" - object does not exist."]}))]
#[case::a_bad_date(json!({"name": "x", "start_date": "tomorrow"}), json!({"start_date": ["Date has wrong format. Use one of these formats instead: YYYY-MM-DD."]}))]
#[case::a_datetime_as_a_date(json!({"name": "x", "target_date": "2026-11-01T00:00:00"}), json!({"target_date": ["Date has wrong format. Use one of these formats instead: YYYY-MM-DD."]}))]
#[case::point_too_big(json!({"name": "x", "point": 13}), json!({"point": ["Ensure this value is less than or equal to 12."]}))]
#[case::point_negative(json!({"name": "x", "point": -1}), json!({"point": ["Ensure this value is greater than or equal to 0."]}))]
#[case::point_text(json!({"name": "x", "point": "many"}), json!({"point": ["A valid integer is required."]}))]
#[case::a_bad_datetime(json!({"name": "x", "deleted_at": "yesterday"}), json!({"deleted_at": ["Datetime has wrong format. Use one of these formats instead: YYYY-MM-DDThh:mm[:ss[.uuuuuu]][+HH:MM|-HH:MM|Z]."]}))]
#[case::a_bad_flag(json!({"name": "x", "is_draft": "maybe"}), json!({"is_draft": ["Must be a valid boolean."]}))]
#[case::a_bad_sequence(json!({"name": "x", "sequence_id": "x"}), json!({"sequence_id": ["A valid integer is required."]}))]
#[case::a_bad_place(json!({"name": "x", "sort_order": "x"}), json!({"sort_order": ["A valid number is required."]}))]
#[case::a_bad_archive_date(json!({"name": "x", "archived_at": 5}), json!({"archived_at": ["Date has wrong format. Use one of these formats instead: YYYY-MM-DD."]}))]
#[case::unknown_maker(json!({"name": "x", "created_by": "5a1b3c4d-0000-4000-8000-000000000001"}), json!({"created_by": ["Invalid pk \"5a1b3c4d-0000-4000-8000-000000000001\" - object does not exist."]}))]
#[case::a_long_source(json!({"name": "x", "external_source": "s".repeat(256)}), json!({"external_source": ["Ensure this field has no more than 255 characters."]}))]
#[case::several(json!({"priority": "x", "point": "y", "start_date": "z"}), json!({"name": ["This field is required."], "priority": ["\"x\" is not a valid choice."], "point": ["A valid integer is required."], "start_date": ["Date has wrong format. Use one of these formats instead: YYYY-MM-DD."]}))]
fn a_creation_with_a_field_wrong_is_refused_with_drfs_message(#[case] body: Value, #[case] errors: Value) {
    let w = real_world();
    let r = call(&w, Who::Admin, "POST", &items("work-items"), &body);
    assert_eq!((r.status, &r.body), (400, &errors));
    assert!(r.has_rate_headers());
    assert!(listed(&w).is_empty(), "nothing was made");
}

/// `validate()` raises at its first refusal, as a non-field error, once every field was fine.
#[test]
fn validate_refuses_what_the_fields_cannot_see() {
    let w = real_world();
    let refused = |body: Value| {
        let r = call(&w, Who::Admin, "POST", &items("work-items"), &body);
        assert_eq!(r.status, 400, "{}", r.body);
        r.body
    };
    assert_eq!(
        refused(json!({"name": "x", "start_date": "2026-11-05", "target_date": "2026-11-01"})),
        json!({"non_field_errors": ["Start date cannot exceed target date"]})
    );
    // a person who is a guest of the project is no one to assign, and neither is a user of no project
    for who in [Who::Guest, Who::Outsider] {
        let want = format!("Assignees [UUID('{}')] are not active members of this project", who.id().0);
        assert_eq!(refused(json!({"name": "x", "assignees": [who.id().0]})), json!({"non_field_errors": [want]}));
    }
    // an empty description has no document to read; one the cleaner answers is kept as it cleans it
    assert_eq!(
        refused(json!({"name": "x", "description_html": ""})),
        json!({"non_field_errors": ["Invalid HTML passed"]})
    );
    let cleaned =
        make(&w, Who::Admin, json!({"name": "clean", "description_html": "<p>a</p><script>alert(1)</script>"}));
    assert_eq!(cleaned["description_html"], json!("<p>a</p>"));
    // a field that is wrong hides `validate()`'s
    assert_eq!(
        refused(json!({"name": "x", "priority": "z", "start_date": "2026-11-05", "target_date": "2026-11-01"})),
        json!({"priority": ["\"z\" is not a valid choice."]})
    );
    // and `validate()` raises at the first of its own: the dates, then the description, then the people
    let guest = Who::Guest.id().0;
    assert_eq!(
        refused(json!({
            "name": "x", "start_date": "2026-11-05", "target_date": "2026-11-01", "description_html": "", "assignees": [guest]
        })),
        json!({"non_field_errors": ["Start date cannot exceed target date"]})
    );
    assert_eq!(
        refused(json!({"name": "x", "description_html": "", "assignees": [guest]})),
        json!({"non_field_errors": ["Invalid HTML passed"]})
    );
    assert!(listed(&w).len() == 1);
}

/// A work item may start the day it is due, not after it: the one day either side of the equal.
#[rstest]
fn a_start_date_may_be_the_target_date_but_not_a_later_day(#[values(-1, 0, 1)] days_after_start: i32) {
    let w = real_world();
    let target = format!("2026-11-{:02}", 5 + days_after_start);
    let r = call(
        &w,
        Who::Admin,
        "POST",
        &items("work-items"),
        &json!({"name": "x", "start_date": "2026-11-05", "target_date": target}),
    );
    if days_after_start < 0 {
        assert_eq!(
            (r.status, r.body),
            (400, json!({"non_field_errors": ["Start date cannot exceed target date"]})),
            "{target}"
        );
        assert!(listed(&w).is_empty());
    } else {
        assert_eq!((r.status, r.body["target_date"].clone()), (201, json!(target)), "{target}");
    }
}

/// A key that a relation cannot read is `“x” is not a valid UUID.` under its own field, and the other fields' errors come beside it:
/// DRF catches Django's `ValidationError` per field. Every answer here is the pinned Plane's, measured on a running one (reading the
/// source had suggested the error escapes the field and makes a bare `Please provide valid detail`, which it does not).
#[rstest]
#[case::state(json!({"name": "x", "state": "nope"}), json!({"state": ["“nope” is not a valid UUID."]}))]
#[case::parent(json!({"name": "x", "parent": 1.5}), json!({"parent": ["“1.5” is not a valid UUID."]}))]
#[case::a_label(json!({"name": "x", "labels": ["nope"]}), json!({"labels": {"0": ["“nope” is not a valid UUID."]}}))]
#[case::an_assignee(json!({"name": "x", "assignees": [["nope"]]}), json!({"assignees": {"0": ["“['nope']” is not a valid UUID."]}}))]
#[case::a_type(json!({"name": "x", "type_id": "nope"}), json!({"type_id": ["“nope” is not a valid UUID."]}))]
#[case::a_type_by_its_model_name(json!({"name": "x", "type": "nope"}), json!({"type": ["“nope” is not a valid UUID."]}))]
#[case::an_estimate(json!({"name": "x", "estimate_point": "nope"}), json!({"estimate_point": ["“nope” is not a valid UUID."]}))]
#[case::the_maker(json!({"name": "x", "created_by": "nope"}), json!({"created_by": ["“nope” is not a valid UUID."]}))]
fn a_key_the_column_cannot_read_is_refused_under_its_field_beside_the_other_fields(
    #[case] body: Value,
    #[case] refusal: Value,
) {
    let w = real_world();
    let mut b = body.as_object().unwrap().clone();
    b.insert("priority".into(), json!("wrong"));
    let mut want = refusal.as_object().unwrap().clone();
    want.insert("priority".into(), json!(["\"wrong\" is not a valid choice."]));
    let r = call(&w, Who::Admin, "POST", &items("work-items"), &Value::Object(b));
    assert_eq!((r.status, r.body), (400, Value::Object(want)));
    assert!(listed(&w).is_empty());
}

/// The corpus's `create-external-id` and `create-external-id-duplicate`.
#[test]
fn a_key_that_another_work_item_has_is_a_409_naming_it() {
    let w = real_world();
    let first = make(&w, Who::Admin, json!({"name": "Ext item", "external_id": "ext-1", "external_source": "oracle"}));
    assert_eq!((first["external_id"].clone(), first["external_source"].clone()), (json!("ext-1"), json!("oracle")));
    let r = call(
        &w,
        Who::Admin,
        "POST",
        &items("work-items"),
        &json!({"name": "again", "external_id": "ext-1", "external_source": "oracle"}),
    );
    assert_eq!(
        (r.status, r.body),
        (
            409,
            json!({"error": "Issue with the same external id and external source already exists", "id": first["id"]})
        )
    );
    for body in [
        json!({"name": "a", "external_id": "ext-1", "external_source": "other"}),
        json!({"name": "b", "external_id": "ext-2", "external_source": "oracle"}),
        json!({"name": "c", "external_id": "ext-1"}),
        json!({"name": "d", "external_source": "oracle"}),
    ] {
        assert_eq!(call(&w, Who::Admin, "POST", &items("work-items"), &body).status, 201, "{body}");
    }
}

/// A body that gives `deleted_at` a value fails in the pin after it has made the work item (PQ-404, EX-001); here it makes none.
#[test]
fn a_deleted_at_on_a_creation_is_the_pins_500_and_makes_nothing() {
    let w = real_world();
    let r =
        call(&w, Who::Admin, "POST", &items("work-items"), &json!({"name": "x", "deleted_at": "2026-11-01T00:00:00Z"}));
    assert_eq!((r.status, r.body.clone()), (500, json!({"error": PLANE_500})));
    assert!(r.has_rate_headers());
    assert!(listed(&w).is_empty());
    // null is no value, and the number the next one has is the first
    let ok = make(&w, Who::Admin, json!({"name": "y", "deleted_at": null}));
    assert_eq!(ok["sequence_id"], json!(1));
}

/// What the pin sets itself, or has no row for, is read and checked and not kept.
#[test]
fn what_is_read_and_not_kept_is_not_kept() {
    let w = real_world();
    let b = make(
        &w,
        Who::Member,
        json!({
            "name": "x", "created_by": Who::Admin.id().0, "created_at": "2020-01-01T00:00:00Z", "is_draft": true,
            "archived_at": "2020-01-01", "sequence_id": 99, "sort_order": 5, "completed_at": "2020-01-01T00:00:00Z",
            "id": id(0xFEED), "workspace": id(0xBAD), "project": id(0xBAD), "updated_by": Who::Admin.id().0,
        }),
    );
    assert_eq!(b["created_by"], json!(Who::Member.id().0), "who the key is for made it");
    assert_eq!(
        (b["sequence_id"].clone(), b["sort_order"].clone(), b["is_draft"].clone(), b["archived_at"].clone()),
        (json!(1), json!(65535.0), json!(false), Value::Null)
    );
    assert_ne!(b["id"], json!(id(0xFEED)));
    assert_eq!(
        (b["workspace"].clone(), b["project"].clone(), b["updated_by"].clone(), b["completed_at"].clone()),
        (json!(id(WS)), json!(id(PROJECT)), Value::Null, Value::Null)
    );
    assert!(
        b["created_at"].as_str().unwrap().starts_with("20") && !b["created_at"].as_str().unwrap().starts_with("2020")
    );
}

/// A body that is not an object: a list is DRF's "Expected a dictionary" 400; the other cases are the same as for projects.
#[test]
fn a_body_that_is_not_an_object_is_refused() {
    let w = real_world();
    let r = raw(&w, Who::Admin, "POST", &items("work-items"), "application/json", "[1]");
    assert_eq!(
        (r.status, r.body),
        (400, json!({"non_field_errors": ["Invalid data. Expected a dictionary, but got list."]}))
    );
    assert_eq!(raw(&w, Who::Admin, "POST", &items("work-items"), "text/plain", "name=x").status, 415);
    assert_eq!(raw(&w, Who::Admin, "POST", &items("work-items"), "application/json", "{bad").status, 400);
    assert_eq!(
        raw(&w, Who::Admin, "POST", &items("work-items"), "application/json", "\"s\"").body,
        json!({"detail": "Request body must be a JSON object or array."})
    );
    let empty = send_raw(&w.router, "POST", &items("work-items"), &[("x-api-key", Who::Admin.key())], Vec::new());
    assert_eq!(empty.body, json!({"name": ["This field is required."]}));
}

// --- who may -------------------------------------------------------------------------------------------------------------------

/// Any member of the project reads every work item, a guest too; an admin or a member writes, a guest does not (nor the guest who
/// made one); anyone else is refused, a refusal is not counted, and a workspace admin who is not in the project is refused too.
#[test]
fn any_member_reads_and_an_admin_or_a_member_writes() {
    let w = real_world();
    let made = make(&w, Who::Admin, json!({"name": "the admin's"}));
    let one = item("work-items", id_of(&made));
    for (who, reads) in
        [(Who::Admin, true), (Who::Member, true), (Who::Guest, true), (Who::Outsider, false), (Who::Inactive, false)]
    {
        let list = get(&w, who, &items("work-items"));
        let detail = get(&w, who, &one);
        assert_eq!((list.status, detail.status), if reads { (200, 200) } else { (403, 403) }, "{who:?} reads");
        if !reads {
            // a switched-off account is a key that is not valid; a stranger is a permission's refusal
            let refusal = if who == Who::Inactive { "Given API token is not valid" } else { DENIED };
            assert_eq!(list.body, json!({"detail": refusal}), "{who:?}");
            assert!(!list.has_rate_headers(), "a refusal is not counted");
        }
    }
    for (who, writes) in
        [(Who::Admin, true), (Who::Member, true), (Who::Guest, false), (Who::Outsider, false), (Who::Inactive, false)]
    {
        let created = call(&w, who, "POST", &items("work-items"), &json!({"name": format!("by {who:?}")}));
        let patched = call(&w, who, "PATCH", &one, &json!({"priority": "low"}));
        assert_eq!((created.status == 201, patched.status == 200), (writes, writes), "{who:?} writes");
        if !writes {
            let refusal = if who == Who::Inactive { "Given API token is not valid" } else { DENIED };
            assert_eq!((created.status, created.body.clone()), (403, json!({"detail": refusal})), "{who:?}");
            assert!(!created.has_rate_headers() && !patched.has_rate_headers());
        }
    }
    // a project the member made: the workspace admin is not in it, so is refused whatever they ask
    let other = project_of(&w, Who::Member, "MEM");
    let there = format!("{}work-items/", project_uri(other));
    assert_eq!(get(&w, Who::Admin, &there).status, 403);
    assert_eq!(call(&w, Who::Admin, "POST", &there, &json!({"name": "no"})).status, 403);
    assert_eq!(call(&w, Who::Member, "POST", &there, &json!({"name": "yes"})).status, 201);
    // and a project that is not there is refused to everyone
    let nowhere = format!("{}work-items/", project_uri(0xBAD));
    assert_eq!(get(&w, Who::Admin, &nowhere).status, 403);
}

// --- the list ------------------------------------------------------------------------------------------------------------------

/// The corpus's `list`: newest first, in the 12-key envelope; both families the same; a guest reads every work item.
#[rstest]
fn the_list_is_newest_first_in_the_pins_envelope(#[values("issues", "work-items")] family: &str) {
    let w = real_world();
    for name in ["one", "two", "three"] {
        make(&w, Who::Admin, json!({"name": name}));
    }
    for who in [Who::Admin, Who::Guest] {
        let r = get(&w, who, &items(family));
        assert_eq!(r.status, 200, "{who:?}");
        assert!(r.has_rate_headers());
        assert_eq!(names(&r), ["three", "two", "one"]);
        assert_eq!(keys(&r.body["results"][0]), set(&KEYS));
        assert_eq!(
            (r.body["total_count"].clone(), r.body["total_results"].clone(), r.body["count"].clone()),
            (json!(3), json!(3), json!(3))
        );
        assert_eq!(
            (r.body["next_cursor"].clone(), r.body["prev_cursor"].clone()),
            (json!("1000:1:0"), json!("1000:-1:1"))
        );
        assert_eq!(
            (r.body["next_page_results"].clone(), r.body["prev_page_results"].clone()),
            (json!(false), json!(false))
        );
    }
}

#[test]
fn the_list_is_paged_and_a_page_asked_wrongly_is_the_pins_refusal() {
    let w = real_world();
    for name in ["one", "two", "three"] {
        make(&w, Who::Admin, json!({"name": name}));
    }
    let first = get(&w, Who::Admin, &format!("{}?per_page=2", items("work-items")));
    assert_eq!(
        (names(&first), first.body["next_page_results"].clone(), first.body["total_pages"].clone()),
        (vec!["three".to_string(), "two".to_string()], json!(true), json!(2))
    );
    let second = get(
        &w,
        Who::Admin,
        &format!("{}?per_page=2&cursor={}", items("work-items"), first.body["next_cursor"].as_str().unwrap()),
    );
    assert_eq!((names(&second), second.body["next_page_results"].clone()), (vec!["one".to_string()], json!(false)));
    for (q, status) in [
        ("per_page=abc", 400),
        ("per_page=1001", 400),
        ("cursor=garbage", 400),
        ("per_page=0", 500),
        ("per_page=-1", 500),
    ] {
        let r = get(&w, Who::Admin, &format!("{}?{q}", items("work-items")));
        assert_eq!(r.status, status, "{q}");
        assert!(r.has_rate_headers(), "{q}");
    }
}

/// The corpus's `list-fields-projection` and `list-expand-assignees-labels-state`.
#[test]
fn fields_cut_the_body_and_expand_swaps_state_assignees_and_labels_for_objects() {
    let w = real_world();
    make(
        &w,
        Who::Admin,
        json!({"name": "x", "state": id(STATE_TODO), "assignees": [Who::Member.id().0], "labels": [id(LABEL_BUG)]}),
    );
    let r = get(&w, Who::Admin, &format!("{}?fields=id,name,sequence_id", items("work-items")));
    assert_eq!(keys(&r.body["results"][0]), set(&["id", "name", "sequence_id"]));
    let r = get(&w, Who::Admin, &format!("{}?expand=state,assignees,labels", items("work-items")));
    let b = &r.body["results"][0];
    assert_eq!(keys(b), set(&KEYS));
    assert_eq!(b["state"], json!({"id": id(STATE_TODO), "name": "Todo", "group": "unstarted", "color": "#000000"}));
    assert_eq!(
        keys(&b["assignees"][0]),
        set(&["id", "first_name", "last_name", "display_name", "email", "avatar", "avatar_url"])
    );
    assert_eq!(b["assignees"][0]["id"], json!(Who::Member.id().0));
    assert_eq!(
        keys(&b["labels"][0]),
        set(&[
            "id",
            "name",
            "color",
            "description",
            "created_at",
            "updated_at",
            "created_by",
            "updated_by",
            "deleted_at",
            "parent",
            "project",
            "sort_order",
            "workspace",
            "external_id",
            "external_source"
        ])
    );
    assert_eq!((b["labels"][0]["name"].clone(), b["labels"][0]["id"].clone()), (json!("bug"), json!(id(LABEL_BUG))));
    // the other names the base serializer expands are not expanded here, and each is ignored
    let r = get(&w, Who::Admin, &format!("{}?expand=project,parent,created_by,nope", items("work-items")));
    assert_eq!((r.status, r.body["results"][0]["project"].clone()), (200, json!(id(PROJECT))));
}

/// The corpus's `list-order-by-sequence` and `list-order-by-priority-desc`: `-priority` runs from `none`, `priority` from `urgent`;
/// a name outside the allowlist, and an order the adapters do not have, are the default.
#[test]
fn order_by_follows_the_pins_allowlist() {
    let w = real_world();
    for (name, priority) in [("a", "none"), ("b", "urgent"), ("c", "low"), ("d", "high"), ("e", "medium")] {
        make(&w, Who::Admin, json!({"name": name, "priority": priority}));
    }
    let order = |q: &str| names(&get(&w, Who::Admin, &format!("{}?order_by={q}", items("work-items"))));
    assert_eq!(order("sequence_id"), ["a", "b", "c", "d", "e"]);
    assert_eq!(order("-sequence_id"), ["e", "d", "c", "b", "a"]);
    assert_eq!(order("-priority"), ["a", "c", "e", "d", "b"], "none, low, medium, high, urgent");
    assert_eq!(order("priority"), ["b", "d", "e", "c", "a"], "urgent first");
    assert_eq!(order("-created_at"), ["e", "d", "c", "b", "a"]);
    // the default runs newest first, so an order is told from it by running the other way: oldest first
    for oldest_first in ["created_at", "updated_at", "sort_order"] {
        assert_eq!(
            order(oldest_first),
            ["a", "b", "c", "d", "e"],
            "{oldest_first}: each was made, and placed, after the last"
        );
        assert_eq!(order(&format!("-{oldest_first}")), ["e", "d", "c", "b", "a"], "-{oldest_first}");
    }
    // `archived_at` is in the allowlist, but a list is of the work items that are not archived: nothing to put in another order
    for default in [
        "name",
        "labels__name",
        "-assignees__first_name",
        "issue_module__module__name",
        "project__name",
        "",
        "archived_at",
        "-archived_at",
    ] {
        assert_eq!(order(default), ["e", "d", "c", "b", "a"], "{default:?} is the default order");
    }
}

/// The three kinds of state that the cast can give a work item (it has a backlog and a to-do state, and a done one is made).
#[derive(Clone, Copy, Debug)]
enum Kind {
    Backlog,
    Todo,
    Done,
}

proptest! {
    #![proptest_config(proptest_config(48))]

    /// `target_date` and `start_date` put the work items that have one in date order, the way the order runs (the same day: newest
    /// first), and those that have none after them, whichever way it runs.
    #[test]
    fn prop_a_date_order_puts_the_dates_in_order_and_the_work_items_without_one_last(
        days in proptest::collection::vec(proptest::option::of(0u32..28), 1..6),
        field in prop_oneof![Just("target_date"), Just("start_date")],
        descending in any::<bool>(),
    ) {
        let w = real_world();
        for (n, day) in days.iter().enumerate() {
            let mut body = json!({"name": format!("w{n}")});
            if let Some(d) = day {
                body[field] = json!(format!("2026-11-{:02}", d + 1));
            }
            make(&w, Who::Admin, body);
        }
        let mut want: Vec<(usize, Option<u32>)> = days.iter().copied().enumerate().collect();
        want.sort_by(|(n, x), (m, y)| match (x, y) {
            (Some(x), Some(y)) if descending => y.cmp(x).then(m.cmp(n)),
            (Some(x), Some(y)) => x.cmp(y).then(m.cmp(n)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => m.cmp(n),
        });
        let order = format!("{}{field}", if descending { "-" } else { "" });
        let got = names(&get(&w, Who::Admin, &format!("{}?order_by={order}", items("work-items"))));
        let want: Vec<String> = want.iter().map(|(n, _)| format!("w{n}")).collect();
        prop_assert_eq!(got, want, "{}", order);
    }

    /// `state__group` and `state__name` both put the work items in the order of their state's group (backlog, unstarted, completed),
    /// whichever way they are asked, and `completed_at` those that are done before those that are not (all done at the one instant here);
    /// within one, newest first.
    #[test]
    fn prop_the_state_and_the_completion_order_the_work_items_by_group_and_by_being_done(
        kinds in proptest::collection::vec(
            prop_oneof![Just(Kind::Backlog), Just(Kind::Todo), Just(Kind::Done)],
            1..7,
        ),
        order in prop_oneof![
            Just("state__group"), Just("-state__group"), Just("state__name"), Just("-state__name"),
            Just("completed_at"), Just("-completed_at"),
        ],
    ) {
        let w = real_world();
        let done = call(&w, Who::Admin, "POST", &format!("{}states/", project_uri(PROJECT)),
            &json!({"name": "Done", "color": "#00aa00", "group": "completed"}));
        prop_assert_eq!(done.status, 200);
        for (n, kind) in kinds.iter().enumerate() {
            let state = match kind {
                Kind::Backlog => backlog(),
                Kind::Todo => json!(id(STATE_TODO)),
                Kind::Done => done.body["id"].clone(),
            };
            make(&w, Who::Admin, json!({"name": format!("w{n}"), "state": state}));
        }
        let rank = |kind: Kind| match (order.ends_with("completed_at"), kind) {
            (true, Kind::Done) => 0,
            (true, _) => 1,
            (false, Kind::Backlog) => 0,
            (false, Kind::Todo) => 1,
            (false, Kind::Done) => 2,
        };
        let mut want: Vec<(usize, Kind)> = kinds.iter().copied().enumerate().collect();
        want.sort_by(|(n, x), (m, y)| rank(*x).cmp(&rank(*y)).then(m.cmp(n)));
        let got = names(&get(&w, Who::Admin, &format!("{}?order_by={order}", items("work-items"))));
        let want: Vec<String> = want.iter().map(|(n, _)| format!("w{n}")).collect();
        prop_assert_eq!(got, want, "{}", order);
    }
}

#[test]
fn what_the_list_does_not_answer_is_refused_in_the_pins_words() {
    let w = real_world();
    make(&w, Who::Admin, json!({"name": "x"}));
    for param in ["pql=state:started", "filters=%7B%7D"] {
        let r = get(&w, Who::Admin, &format!("{}?{param}", items("work-items")));
        assert_eq!(r.status, 400, "{param}");
        assert_eq!(r.body["unsupported_parameters"], json!([param.split('=').next().unwrap()]));
        assert!(r.body["pql"].as_str().unwrap().starts_with("PQL and structured filters are not supported"));
    }
    // both together, and an empty one is none
    let r = get(&w, Who::Admin, &format!("{}?pql=a&filters=b", items("work-items")));
    assert_eq!(r.body["unsupported_parameters"], json!(["pql", "filters"]));
    assert_eq!(get(&w, Who::Admin, &format!("{}?pql=&filters=", items("work-items"))).status, 200);
}

/// With both halves of a key the list answers the one work item that has it, not a page; one that is nowhere is a 404.
#[test]
fn a_key_in_the_query_makes_the_list_the_one_work_item_that_has_it() {
    let w = real_world();
    let a = make(&w, Who::Admin, json!({"name": "a", "external_id": "1", "external_source": "gh"}));
    make(&w, Who::Admin, json!({"name": "b"}));
    let one = get(&w, Who::Guest, &format!("{}?external_id=1&external_source=gh", items("work-items")));
    assert_eq!((one.status, one.body["id"].clone()), (200, a["id"].clone()));
    assert!(one.body.get("results").is_none());
    let cut = get(&w, Who::Admin, &format!("{}?external_id=1&external_source=gh&fields=name", items("work-items")));
    assert_eq!(cut.body, json!({"name": "a"}));
    let missing = get(&w, Who::Admin, &format!("{}?external_id=2&external_source=gh", items("work-items")));
    assert_eq!((missing.status, missing.body), (404, json!({"error": NOT_FOUND})));
    // half a key is no key
    assert_eq!(get(&w, Who::Admin, &format!("{}?external_id=2", items("work-items"))).body["total_count"], json!(2));
}

#[test]
fn an_archived_project_has_no_work_items_for_a_read_but_a_change_and_a_creation_still_work() {
    let w = real_world();
    let a = make(&w, Who::Admin, json!({"name": "a"}));
    let one = item("work-items", id_of(&a));
    archive(&w);
    let list = get(&w, Who::Admin, &items("work-items"));
    assert_eq!(
        (list.status, list.body["total_count"].clone(), list.body["results"].clone()),
        (200, json!(0), json!([]))
    );
    assert_eq!(get(&w, Who::Admin, &one).status, 404);
    let patched = call(&w, Who::Admin, "PATCH", &one, &json!({"name": "renamed"}));
    assert_eq!((patched.status, patched.body["name"].clone()), (200, json!("renamed")));
    assert_eq!(call(&w, Who::Admin, "POST", &items("work-items"), &json!({"name": "during"})).status, 201);
    assert_eq!(call(&w, Who::Admin, "DELETE", &one, &json!({})).status, 204);
}

// --- one work item ---------------------------------------------------------------------------------------------------------------

#[rstest]
fn a_work_item_is_read_by_its_id_in_either_family(#[values("issues", "work-items")] family: &str) {
    let w = real_world();
    let a = make(&w, Who::Admin, json!({"name": "a", "assignees": [Who::Member.id().0]}));
    let r = get(&w, Who::Guest, &item(family, id_of(&a)));
    assert_eq!((r.status, r.body.clone()), (200, a.clone()));
    assert!(r.has_rate_headers());
    let cut = get(&w, Who::Admin, &format!("{}?fields=id,priority&expand=assignees", item(family, id_of(&a))));
    assert_eq!(cut.body, json!({"id": a["id"], "priority": "none"}));
    let expanded = get(&w, Who::Admin, &format!("{}?expand=assignees", item(family, id_of(&a))));
    assert_eq!(expanded.body["assignees"][0]["id"], json!(Who::Member.id().0));
    // what is nowhere, and what is no id
    for (uri, body) in [
        (item(family, id(0xBAD).hyphenated()), json!({"error": NOT_FOUND})),
        (item(family, "nope"), json!({"error": "Page not found."})),
    ] {
        let r = get(&w, Who::Admin, &uri);
        assert_eq!((r.status, r.body), (404, body), "{uri}");
    }
}

/// The corpus's `detail-by-identifier` and `detail-by-identifier-unknown`.
#[rstest]
fn a_work_item_is_found_by_the_identifier_of_its_project_and_its_number(
    #[values("issues", "work-items")] family: &str,
) {
    let w = real_world();
    let a = make(&w, Who::Admin, json!({"name": "a"}));
    let key = identifier(&w);
    let by = |who: Who, ident: &str| get(&w, who, &format!("/api/v1/workspaces/{SLUG}/{family}/{ident}/"));
    for who in [Who::Admin, Who::Guest] {
        let r = by(who, &format!("{key}-1"));
        assert_eq!((r.status, r.body.clone()), (200, a.clone()), "{who:?}");
        assert!(r.has_rate_headers());
    }
    let cut = get(&w, Who::Admin, &format!("/api/v1/workspaces/{SLUG}/{family}/{key}-1/?fields=name"));
    assert_eq!(cut.body, json!({"name": "a"}));
    // a number nobody has, and one that is no number
    let missing = by(Who::Admin, &format!("{key}-9999"));
    assert_eq!((missing.status, missing.body), (404, json!({"error": NOT_FOUND})));
    let nan = by(Who::Admin, &format!("{key}-one"));
    assert_eq!((nan.status, nan.body), (404, json!({"error": "Work item not found"})));
    // the route splits at the last hyphen: an identifier with one is another project's
    assert_eq!(by(Who::Admin, &format!("{key}-extra-1")).status, 403);
    // with nothing before the last hyphen, nothing after it, or no hyphen at all, there is no such route
    for ident in ["-1".to_string(), format!("{key}-"), "nohyphen".to_string()] {
        let r = by(Who::Admin, &ident);
        assert_eq!((r.status, r.body.clone()), (404, json!({"error": "Page not found."})), "{ident}");
    }
    // an identifier that is nobody's, or whose project the caller is not in, is a permission's refusal, and so is a stranger
    for (who, ident) in [(Who::Admin, "NOPE-1".to_string()), (Who::Outsider, format!("{key}-1"))] {
        let r = by(who, &ident);
        assert_eq!((r.status, r.body.clone()), (403, json!({"detail": DENIED})), "{who:?} {ident}");
        assert!(!r.has_rate_headers());
    }
    // a switched-off account's key is not a valid key
    let r = by(Who::Inactive, &format!("{key}-1"));
    assert_eq!((r.status, r.body), (403, json!({"detail": "Given API token is not valid"})));
    // no hyphen is no route
    let r = by(Who::Admin, "nohyphen");
    assert_eq!((r.status, r.body), (404, json!({"error": "Page not found."})));
}

// --- the search ----------------------------------------------------------------------------------------------------------------

fn search(w: &World, who: Who, family: &str, query: &str) -> Reply {
    get(w, who, &format!("/api/v1/workspaces/{SLUG}/{family}/search/?{query}"))
}

fn hit_names(r: &Reply) -> Vec<String> {
    r.body["issues"].as_array().unwrap().iter().map(|h| h["name"].as_str().unwrap().to_string()).collect()
}

/// The corpus's `search`: the pin's wrapper, newest first, what a hit says, and what the text matches.
#[rstest]
fn a_search_finds_what_a_name_an_identifier_or_a_number_says(#[values("issues", "work-items")] family: &str) {
    let w = real_world();
    let (login, signup) = (
        make(&w, Who::Admin, json!({"name": "Fix the Login page"})),
        make(&w, Who::Admin, json!({"name": "Sign-up form"})),
    );
    make(&w, Who::Admin, json!({"name": "100% done_ish"}));
    let r = search(&w, Who::Member, family, "search=LOGIN");
    assert_eq!(r.status, 200);
    assert!(r.has_rate_headers());
    assert_eq!(hit_names(&r), ["Fix the Login page"]);
    let hit = &r.body["issues"][0];
    assert_eq!(keys(hit), set(&["id", "name", "sequence_id", "project__identifier", "project_id", "workspace__slug"]));
    assert_eq!(
        (hit["id"].clone(), hit["sequence_id"].clone(), hit["project_id"].clone(), hit["workspace__slug"].clone()),
        (login["id"].clone(), json!(1), json!(id(PROJECT)), json!(SLUG))
    );
    assert_eq!(hit["project__identifier"], json!(identifier(&w)));
    // a number in the text is a work item's number, and the identifier finds every work item of the project, newest first
    assert_eq!(hit_names(&search(&w, Who::Member, family, "search=zzz%202")), ["Sign-up form"]);
    assert_eq!(search(&w, Who::Member, family, "search=zzz%202").body["issues"][0]["id"], signup["id"]);
    let key = identifier(&w).to_lowercase();
    assert_eq!(
        hit_names(&search(&w, Who::Member, family, &format!("search={key}"))),
        ["100% done_ish", "Sign-up form", "Fix the Login page"]
    );
    // `%` and `_` are ordinary characters, a limit cuts, and the project narrows
    assert_eq!(hit_names(&search(&w, Who::Member, family, "search=100%25")), ["100% done_ish"]);
    assert_eq!(hit_names(&search(&w, Who::Member, family, "search=a_b")), Vec::<String>::new());
    assert_eq!(hit_names(&search(&w, Who::Member, family, &format!("search={key}&limit=1"))), ["100% done_ish"]);
    assert_eq!(
        hit_names(&search(&w, Who::Member, family, &format!("search=login&project_id={}", id(PROJECT).hyphenated()))),
        ["Fix the Login page"]
    );
    assert!(hit_names(&search(
        &w,
        Who::Member,
        family,
        &format!("search=login&project_id={}", id(0xBAD).hyphenated())
    ))
    .is_empty());
    // `workspace_search` other than `false` ignores the project
    assert_eq!(
        hit_names(&search(
            &w,
            Who::Member,
            family,
            &format!("search=login&workspace_search=true&project_id={}", id(0xBAD).hyphenated())
        )),
        ["Fix the Login page"]
    );
}

#[test]
fn a_search_that_asks_for_nothing_or_is_asked_wrongly_answers_as_the_pin_does() {
    let w = real_world();
    make(&w, Who::Admin, json!({"name": "login"}));
    for q in ["", "search=", "limit=5", "search=&limit=abc"] {
        let r = search(&w, Who::Admin, "work-items", q);
        assert_eq!((r.status, r.body), (200, json!({"issues": []})), "{q:?}");
    }
    // `int(limit)` raises on text, and the ORM refuses a negative slice: the pin's 500; nought is no rows
    for q in ["search=login&limit=abc", "search=login&limit=-1", "search=login&limit=1.5"] {
        let r = search(&w, Who::Admin, "work-items", q);
        assert_eq!((r.status, r.body.clone()), (500, json!({"error": PLANE_500})), "{q}");
        assert!(r.has_rate_headers());
    }
    assert_eq!(search(&w, Who::Admin, "work-items", "search=login&limit=0").body, json!({"issues": []}));
    // a project that is no uuid raises in Django's filter
    let r = search(&w, Who::Admin, "work-items", "search=login&project_id=nope");
    assert_eq!((r.status, r.body), (400, json!({"error": "Please provide valid detail"})));
    // someone in the workspace and in no project is answered, not refused
    let r = search(&w, Who::Outsider, "work-items", "search=login");
    assert_eq!((r.status, r.body), (200, json!({"issues": []})));
    // a key that is not valid is the key's refusal
    let r = get(&w, Who::Inactive, &format!("/api/v1/workspaces/{SLUG}/work-items/search/?search=login"));
    assert_eq!((r.status, r.body), (403, json!({"detail": "Given API token is not valid"})));
}

#[test]
fn an_archived_project_is_not_searched() {
    let w = real_world();
    make(&w, Who::Admin, json!({"name": "login"}));
    assert_eq!(hit_names(&search(&w, Who::Admin, "work-items", "search=login")), ["login"]);
    archive(&w);
    assert!(hit_names(&search(&w, Who::Admin, "work-items", "search=login")).is_empty());
}

// --- changing a work item -------------------------------------------------------------------------------------------------------

/// The corpus's `patch`: what the body names changes, the rest stays, who changed it is recorded; and both families change the one.
#[rstest]
fn a_change_sets_what_it_names_and_records_who_changed_it(#[values("issues", "work-items")] family: &str) {
    let w = real_world();
    let a = make(&w, Who::Admin, json!({"name": "Scratch item A", "point": 3, "labels": [id(LABEL_BUG)]}));
    let one = item(family, id_of(&a));
    let r = call(&w, Who::Member, "PATCH", &one, &json!({"name": "Scratch item A2", "priority": "urgent"}));
    assert_eq!(r.status, 200);
    assert!(r.has_rate_headers());
    assert_eq!(keys(&r.body), set(&KEYS));
    let b = &r.body;
    assert_eq!((b["name"].clone(), b["priority"].clone()), (json!("Scratch item A2"), json!("urgent")));
    assert_eq!(
        (b["updated_by"].clone(), b["created_by"].clone()),
        (json!(Who::Member.id().0), json!(Who::Admin.id().0))
    );
    assert_eq!(
        (b["point"].clone(), b["labels"].clone(), b["sequence_id"].clone()),
        (json!(3), json!([id(LABEL_BUG)]), json!(1)),
        "what it did not name stays"
    );
    // the state, the people, the labels, the dates, the point, a parent and a key can be set, and cleared
    let done = call(
        &w,
        Who::Admin,
        "PATCH",
        &one,
        &json!({
            "state": id(STATE_TODO), "assignees": [Who::Member.id().0], "labels": [id(LABEL_FEATURE), id(LABEL_DOCS)],
            "start_date": "2026-11-01", "target_date": "2026-11-09", "point": 12, "external_source": "gh", "external_id": "9",
            "description_html": "<p>new</p>",
        }),
    );
    let d = &done.body;
    assert_eq!(
        (d["state"].clone(), d["assignees"].clone(), d["point"].clone(), d["description_html"].clone()),
        (json!(id(STATE_TODO)), json!([Who::Member.id().0]), json!(12), json!("<p>new</p>"))
    );
    assert_eq!(d["labels"].as_array().unwrap().len(), 2);
    assert_eq!(
        (d["external_source"].clone(), d["external_id"].clone(), d["start_date"].clone()),
        (json!("gh"), json!("9"), json!("2026-11-01"))
    );
    let cleared = call(
        &w,
        Who::Admin,
        "PATCH",
        &one,
        &json!({"assignees": [], "labels": [], "start_date": null, "target_date": null, "point": null, "external_source": null, "external_id": null}),
    );
    let c = &cleared.body;
    assert_eq!(
        (
            c["assignees"].clone(),
            c["labels"].clone(),
            c["start_date"].clone(),
            c["point"].clone(),
            c["external_id"].clone()
        ),
        (json!([]), json!([]), Value::Null, Value::Null, Value::Null)
    );
    // a name alone, and nothing at all, keep the rest
    assert_eq!(call(&w, Who::Admin, "PATCH", &one, &json!({})).status, 200);
    assert_eq!(call(&w, Who::Admin, "PATCH", &one, &json!({"name": "last"})).body["state"], json!(id(STATE_TODO)));
}

/// A parent is a work item of the project, itself too; it can be cleared; the work items under a deleted one go with it.
#[test]
fn a_parent_is_set_cleared_and_deleted_with() {
    let w = real_world();
    let (parent, child, other) = (
        make(&w, Who::Admin, json!({"name": "parent"})),
        make(&w, Who::Admin, json!({"name": "child"})),
        make(&w, Who::Admin, json!({"name": "other"})),
    );
    let child_uri = item("work-items", id_of(&child));
    let r = call(&w, Who::Admin, "PATCH", &child_uri, &json!({"parent": parent["id"]}));
    assert_eq!(r.body["parent"], parent["id"]);
    let made = make(&w, Who::Admin, json!({"name": "made under", "parent": parent["id"]}));
    assert_eq!(made["parent"], parent["id"]);
    assert_eq!(
        call(&w, Who::Admin, "PATCH", &child_uri, &json!({"parent": child["id"]})).body["parent"],
        child["id"],
        "itself, as the pin allows"
    );
    assert_eq!(call(&w, Who::Admin, "PATCH", &child_uri, &json!({"parent": parent["id"]})).status, 200);
    // one that is nowhere is `Invalid pk`, and null clears
    let nowhere = call(&w, Who::Admin, "PATCH", &child_uri, &json!({"parent": id(0xBAD)}));
    assert_eq!(nowhere.body, json!({"parent": [format!("Invalid pk \"{}\" - object does not exist.", id(0xBAD))]}));
    assert_eq!(call(&w, Who::Admin, "PATCH", &child_uri, &json!({"parent": null})).body["parent"], Value::Null);
    call(&w, Who::Admin, "PATCH", &child_uri, &json!({"parent": parent["id"]}));
    // deleting the parent takes what is under it, and nothing else
    assert_eq!(call(&w, Who::Admin, "DELETE", &item("work-items", id_of(&parent)), &json!({})).status, 204);
    assert_eq!(listed(&w), ["other"]);
    assert_eq!(get(&w, Who::Admin, &child_uri).status, 404);
    assert_eq!(get(&w, Who::Admin, &item("work-items", id_of(&other))).status, 200);
}

/// The base view parses the body before the handler runs, so a body that cannot be parsed is refused before the work item is looked
/// for; the handler then finds it, and only then does the serializer read what the parsed body says.
#[test]
fn a_change_parses_the_body_then_finds_its_work_item_then_reads_what_it_says() {
    let w = real_world();
    let a = make(&w, Who::Admin, json!({"name": "a"}));
    let (one, nowhere) = (item("work-items", id_of(&a)), item("work-items", id(0xBAD).hyphenated()));
    for uri in [&nowhere, &one] {
        assert_eq!(raw(&w, Who::Admin, "PATCH", uri, "text/plain", "x").status, 415, "{uri}");
        let bad = raw(&w, Who::Admin, "PATCH", uri, "application/json", "{bad");
        assert_eq!(bad.status, 400, "{uri}");
        assert!(bad.body["detail"].as_str().unwrap().starts_with("JSON parse error - "), "{uri}");
        let scalar = raw(&w, Who::Admin, "PATCH", uri, "application/json", "\"s\"");
        assert_eq!(scalar.body, json!({"detail": "Request body must be a JSON object or array."}), "{uri}");
    }
    let r = raw(&w, Who::Admin, "PATCH", &nowhere, "application/json", "[1]");
    assert_eq!((r.status, r.body), (404, json!({"error": NOT_FOUND})));
    let r = raw(&w, Who::Admin, "PATCH", &one, "application/json", "[1]");
    assert_eq!(r.body, json!({"non_field_errors": ["Invalid data. Expected a dictionary, but got list."]}));
    let r = raw(&w, Who::Admin, "PATCH", &nowhere, "application/json", "{\"priority\": \"x\"}");
    assert_eq!(
        (r.status, r.body),
        (404, json!({"error": NOT_FOUND})),
        "the work item is looked for before its fields are read"
    );
    let r = call(&w, Who::Admin, "PATCH", &one, &json!({"priority": "x"}));
    assert_eq!((r.status, r.body), (400, json!({"priority": ["\"x\" is not a valid choice."]})));
    assert_eq!(raw(&w, Who::Admin, "PATCH", &nowhere, "text/plain", "").status, 404);
}

/// The corpus's `patch-invalid-priority` and `patch-unknown`, and the 409 for a key: it names the work item being changed.
#[test]
fn a_change_with_a_wrong_field_or_for_nothing_is_refused_and_a_key_is_a_409_naming_the_one_changed() {
    let w = real_world();
    let holder = make(&w, Who::Admin, json!({"name": "holder", "external_id": "1", "external_source": "gh"}));
    let a = make(&w, Who::Admin, json!({"name": "a"}));
    let one = item("work-items", id_of(&a));
    let r = call(&w, Who::Admin, "PATCH", &one, &json!({"priority": "super"}));
    assert_eq!((r.status, r.body), (400, json!({"priority": ["\"super\" is not a valid choice."]})));
    // There are no types and no estimates: a key for one is a row that is not there, under the name it was sent as, on a change as on a
    // creation. Found by the conformance gate (a fuzzed change that named a `type`: 400 on the pin, and a 200 here until the field
    // was read); the pin reads the type under both `type` and `type_id`.
    for field in ["type", "type_id", "estimate_point"] {
        let key = "5a1b3c4d-0000-4000-8000-000000000001";
        let r = call(&w, Who::Admin, "PATCH", &one, &json!({ field: key }));
        let want = json!({ field: [format!("Invalid pk \"{key}\" - object does not exist.")] });
        assert_eq!((r.status, r.body), (400, want), "{field}");
    }
    let r = call(&w, Who::Admin, "PATCH", &item("work-items", id(0xBAD).hyphenated()), &json!({"name": "x"}));
    assert_eq!((r.status, r.body), (404, json!({"error": NOT_FOUND})));
    // a dates pair the other way about is `validate()`'s refusal when the body names both
    let r = call(&w, Who::Admin, "PATCH", &one, &json!({"start_date": "2026-11-05", "target_date": "2026-11-01"}));
    assert_eq!(r.body, json!({"non_field_errors": ["Start date cannot exceed target date"]}));
    // a key that the holder has
    let r = call(&w, Who::Admin, "PATCH", &one, &json!({"external_id": "1", "external_source": "gh"}));
    assert_eq!(
        (r.status, r.body),
        (409, json!({"error": "Issue with the same external id and external source already exists", "id": a["id"]}))
    );
    assert_eq!(
        call(&w, Who::Admin, "PATCH", &one, &json!({"external_id": "1"})).status,
        200,
        "no source and none of its own: another key"
    );
    // the holder keeps its own, and is given it again
    let h = item("work-items", id_of(&holder));
    assert_eq!(call(&w, Who::Admin, "PATCH", &h, &json!({"external_id": "1", "external_source": "gh"})).status, 200);
}

// --- deleting a work item -------------------------------------------------------------------------------------------------------

/// The corpus's `delete`, `detail-after-delete` and `delete-again`: 204 with no body, then it is gone, and a second deletion is a 404.
#[rstest]
fn a_work_item_is_deleted_and_then_it_is_gone(#[values("issues", "work-items")] family: &str) {
    let w = real_world();
    let a = make(&w, Who::Admin, json!({"name": "a"}));
    let one = item(family, id_of(&a));
    let r = call(&w, Who::Admin, "DELETE", &one, &json!({}));
    assert_eq!((r.status, r.body.clone()), (204, Value::Null));
    assert!(r.has_rate_headers());
    let gone = get(&w, Who::Admin, &one);
    assert_eq!((gone.status, gone.body), (404, json!({"error": NOT_FOUND})));
    let again = call(&w, Who::Admin, "DELETE", &one, &json!({}));
    assert_eq!((again.status, again.body), (404, json!({"error": NOT_FOUND})));
    assert!(listed(&w).is_empty());
    // the number is not used again
    assert_eq!(make(&w, Who::Admin, json!({"name": "b"}))["sequence_id"], json!(2));
}

/// A project admin deletes any work item and a member only one they made: the refusal is the handler's, so it is counted and has
/// the pin's words; a guest is refused before that, by the permission, and not counted.
#[test]
fn only_a_project_admin_or_the_maker_deletes_a_work_item() {
    let w = real_world();
    let (mine, theirs) =
        (make(&w, Who::Member, json!({"name": "the member's"})), make(&w, Who::Admin, json!({"name": "the admin's"})));
    let r = call(&w, Who::Member, "DELETE", &item("work-items", id_of(&theirs)), &json!({}));
    assert_eq!((r.status, r.body.clone()), (403, json!({"error": "Only admin or creator can delete the work item"})));
    assert!(r.has_rate_headers(), "a handler's refusal is counted");
    assert_eq!(get(&w, Who::Admin, &item("work-items", id_of(&theirs))).status, 200);
    let guest = call(&w, Who::Guest, "DELETE", &item("work-items", id_of(&mine)), &json!({}));
    assert_eq!((guest.status, guest.body.clone()), (403, json!({"detail": DENIED})));
    assert!(!guest.has_rate_headers());
    assert_eq!(call(&w, Who::Member, "DELETE", &item("work-items", id_of(&mine)), &json!({})).status, 204);
    assert_eq!(call(&w, Who::Admin, "DELETE", &item("work-items", id_of(&theirs)), &json!({})).status, 204);
    assert!(listed(&w).is_empty());
}
