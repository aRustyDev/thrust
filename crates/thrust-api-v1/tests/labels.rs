//! The labels of Plane's v1 API: list, create, retrieve, change and delete, through the crate's own router over the memory adapters.
//!
//! What must hold, each pinned by the oracle's corpus (`labels.json`) or read from the pin's source: the body's keys; a read is for any
//! member of the project and a write for a project ADMIN OR MEMBER, so a guest is refused and so is a workspace admin who is not in the
//! project; a create answers 201 and places the label after the greatest one by 10000; the same queryset serves a read, a change and a
//! deletion (an archived project has no labels to find); DRF's field messages in the order of the serializer's fields; the three
//! answers a name that is taken gets; a body that cannot be parsed is refused before the label is looked for; and a deletion takes the
//! labels under the label and the work items' hold on them.
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

/// The keys of a label's body (`LabelSerializer`, `fields = "__all__"`).
const LABEL_KEYS: [&str; 15] = [
    "color",
    "created_at",
    "created_by",
    "deleted_at",
    "description",
    "external_id",
    "external_source",
    "id",
    "name",
    "parent",
    "project",
    "sort_order",
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

fn labels() -> String {
    format!("{}labels/", project_uri(PROJECT))
}

fn label(n: impl std::fmt::Display) -> String {
    format!("{}{n}/", labels())
}

fn get(w: &World, who: Who, uri: &str) -> Reply {
    send_as(&w.router, who.key(), "GET", uri)
}

fn call(w: &World, who: Who, method: &str, uri: &str, body: &Value) -> Reply {
    send_json(&w.router, who.key(), method, uri, body)
}

fn names(r: &Reply) -> Vec<String> {
    r.body["results"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap().to_string()).collect()
}

fn listed(w: &World) -> Vec<String> {
    names(&get(w, Who::Admin, &labels()))
}

fn bug() -> String {
    id(LABEL_BUG).hyphenated().to_string()
}

fn archive(w: &World) {
    assert_eq!(call(w, Who::Admin, "POST", &format!("{}archive/", project_uri(PROJECT)), &json!({})).status, 204);
}

/// A project that `who` makes through core (so they are its admin and the others are not in it): its id.
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

/// A work item of the cast project that carries `labels`, made through core as a member would; its id.
fn item_with(w: &World, labels: &[Uuid]) -> WorkItemId {
    let who = Principal { user_id: Who::Member.id() };
    let access = block_on(issue_writer(&w.app, &who, &WorkspaceSlug::new(SLUG), ProjectId(id(PROJECT)))).unwrap();
    let draft = IssueDraft { name: "an item".into(), label_ids: labels.to_vec(), ..IssueDraft::default() };
    block_on(create_issue(&w.app, &access, draft)).unwrap().item.id
}

fn labels_of(w: &World, item: WorkItemId) -> Vec<Uuid> {
    let who = Principal { user_id: Who::Member.id() };
    let access = block_on(issue_writer(&w.app, &who, &WorkspaceSlug::new(SLUG), ProjectId(id(PROJECT)))).unwrap();
    block_on(get_issue(&w.app, &access, item)).unwrap().attrs.label_ids
}

// --- the body, the list and a label ------------------------------------------------------------------------------------------

/// The corpus's `detail`: the 15 keys, and the values the oracle's seed gives `bug`.
#[test]
fn a_label_has_the_pins_keys_and_values() {
    let w = real_world();
    let r = get(&w, Who::Guest, &label(bug()));
    assert_eq!(r.status, 200);
    assert!(r.has_rate_headers());
    assert_eq!(keys(&r.body), set(&LABEL_KEYS));
    let b = &r.body;
    assert_eq!(
        (b["name"].as_str(), b["color"].as_str(), b["description"].as_str()),
        (Some("bug"), Some("#E5484D"), Some(""))
    );
    assert_eq!(b["sort_order"], json!(65535.0));
    assert_eq!(
        (b["parent"].clone(), b["external_source"].clone(), b["external_id"].clone()),
        (Value::Null, Value::Null, Value::Null)
    );
    assert_eq!(
        (b["created_by"].clone(), b["updated_by"].clone(), b["deleted_at"].clone()),
        (Value::Null, Value::Null, Value::Null)
    );
    assert_eq!((b["project"].clone(), b["workspace"].clone()), (json!(id(PROJECT)), json!(id(WS))));
    assert_eq!(b["id"], json!(id(LABEL_BUG)));
}

/// The corpus's `list`: newest first (docs, feature, bug), the pin's envelope; a guest reads it too, a stranger does not.
#[test]
fn the_list_is_newest_first_with_the_pins_envelope() {
    let w = real_world();
    for who in [Who::Admin, Who::Member, Who::Guest] {
        let r = get(&w, who, &labels());
        assert_eq!(r.status, 200, "{who:?}");
        assert_eq!(names(&r), ["docs", "feature", "bug"]);
        assert_eq!(keys(&r.body["results"][0]), set(&LABEL_KEYS));
        assert_eq!((r.body["total_results"].clone(), r.body["count"].clone()), (json!(3), json!(3)));
        assert_eq!(
            (r.body["next_cursor"].clone(), r.body["prev_cursor"].clone()),
            (json!("1000:1:0"), json!("1000:-1:1"))
        );
    }
    let outsider = get(&w, Who::Outsider, &labels());
    assert_eq!((outsider.status, outsider.body), (403, json!({ "detail": DENIED })));
}

/// `fields` and the pages work on the list; the pin's list has no `order_by` of its own and no `expand`, so both are ignored.
#[test]
fn fields_and_pages_work_on_the_list_and_order_by_and_expand_do_nothing() {
    let w = real_world();
    let r = get(&w, Who::Admin, &format!("{}?fields=id,name", labels()));
    assert_eq!(keys(&r.body["results"][0]), set(&["id", "name"]));
    for ignored in ["order_by=name", "order_by=sort_order", "expand=project,parent,created_by"] {
        let r = get(&w, Who::Admin, &format!("{}?{ignored}", labels()));
        assert_eq!((r.status, names(&r)), (200, vec!["docs".into(), "feature".into(), "bug".into()]), "{ignored}");
        assert_eq!(keys(&r.body["results"][0]), set(&LABEL_KEYS), "{ignored}");
    }
    // pages of two and one
    let first = get(&w, Who::Admin, &format!("{}?per_page=2", labels()));
    assert_eq!(
        (names(&first), first.body["next_page_results"].clone(), first.body["total_pages"].clone()),
        (vec!["docs".to_string(), "feature".to_string()], json!(true), json!(2))
    );
    let second =
        get(&w, Who::Admin, &format!("{}?per_page=2&cursor={}", labels(), first.body["next_cursor"].as_str().unwrap()));
    assert_eq!((names(&second), second.body["next_page_results"].clone()), (vec!["bug".to_string()], json!(false)));
    for (q, status) in [("per_page=abc", 400), ("per_page=1001", 400), ("cursor=garbage", 400), ("per_page=0", 500)] {
        let r = get(&w, Who::Admin, &format!("{}?{q}", labels()));
        assert_eq!(r.status, status, "{q}");
        assert!(r.has_rate_headers(), "{q}");
    }
}

/// The detail read builds its serializer with neither `fields` nor `expand`: both are ignored there (the state detail honours `fields`).
#[test]
fn the_detail_read_ignores_fields_and_expand() {
    let w = real_world();
    for q in ["fields=id,name", "expand=project,created_by", "fields=nope"] {
        let r = get(&w, Who::Admin, &format!("{}?{q}", label(bug())));
        assert_eq!(r.status, 200, "{q}");
        assert_eq!(keys(&r.body), set(&LABEL_KEYS), "{q}");
    }
}

#[test]
fn a_label_that_is_not_there_is_not_found() {
    let w = real_world();
    let other = project_of(&w, Who::Admin, "OTH");
    let theirs = {
        let p = Principal { user_id: Who::Admin.id() };
        let scope =
            block_on(authorize(&w.app, &p, &WorkspaceSlug::new(SLUG), ProjectId(id(other)), Permission::WriteLabel))
                .unwrap();
        block_on(create_label(&w.app, &scope, LabelPatch { name: Some("theirs".into()), ..LabelPatch::default() }))
            .unwrap()
    };
    // one that is nowhere, one of another project of the workspace, and one that is not a uuid
    for (uri, body) in [
        (label(id(0xBAD).hyphenated()), json!({ "error": NOT_FOUND })),
        (label(theirs.id.hyphenated()), json!({ "error": NOT_FOUND })),
        (label("nope"), json!({ "error": "Page not found." })),
    ] {
        let r = get(&w, Who::Admin, &uri);
        assert_eq!((r.status, r.body), (404, body), "{uri}");
    }
    // a project that is not there is a 403, as for every route that asks for membership
    let r = get(&w, Who::Admin, &format!("{}labels/", project_uri(0xBAD)));
    assert_eq!((r.status, r.body), (403, json!({ "detail": DENIED })));
}

/// The queryset of a read, a change and a deletion leaves out an archived project; a creation does not look.
#[test]
fn an_archived_project_has_no_labels_to_find_but_one_can_still_be_made() {
    let w = real_world();
    archive(&w);
    let list = get(&w, Who::Admin, &labels());
    assert_eq!((list.status, names(&list), list.body["total_results"].clone()), (200, Vec::<String>::new(), json!(0)));
    assert_eq!(get(&w, Who::Admin, &label(bug())).status, 404);
    assert_eq!(call(&w, Who::Admin, "PATCH", &label(bug()), &json!({"name": "Still here"})).status, 404);
    assert_eq!(call(&w, Who::Admin, "DELETE", &label(bug()), &json!({})).status, 404);
    let made = call(&w, Who::Admin, "POST", &labels(), &json!({"name": "during"}));
    assert_eq!((made.status, made.body["name"].clone()), (201, json!("during")));
}

// --- who may write ------------------------------------------------------------------------------------------------------------

/// A write is for a project admin or member: a guest is refused, so is a stranger, so is a switched-off account, and so is an admin of
/// the WORKSPACE who is not in the project. A refusal is not counted. The control: the same writes by a member succeed.
#[test]
fn a_write_is_for_a_project_admin_or_member_and_no_one_else() {
    let write = |w: &World, who: Who, project: u128| -> Vec<u16> {
        let base = format!("{}labels/", project_uri(project));
        let created = call(w, who, "POST", &base, &json!({"name": format!("by {who:?}")}));
        let other = format!("{base}{}/", id(LABEL_BUG).hyphenated());
        let patched = call(w, who, "PATCH", &other, &json!({"color": "#fff"}));
        let deleted = call(w, who, "DELETE", &other, &json!({}));
        vec![created.status, patched.status, deleted.status]
    };
    for (who, want) in [
        (Who::Admin, vec![201, 200, 204]),
        (Who::Member, vec![201, 200, 204]),
        (Who::Guest, vec![403, 403, 403]),
        (Who::Outsider, vec![403, 403, 403]),
        (Who::Inactive, vec![403, 403, 403]),
    ] {
        let w = real_world();
        assert_eq!(write(&w, who, PROJECT), want, "{who:?} on the cast project");
    }
    // a project the member made: the workspace admin is not in it, so is refused, and the refusal is not counted
    let w = real_world();
    let project = project_of(&w, Who::Member, "MEM");
    let base = format!("{}labels/", project_uri(project));
    let admin = call(&w, Who::Admin, "POST", &base, &json!({"name": "Nope"}));
    assert_eq!(
        (admin.status, admin.body.clone()),
        (403, json!({ "detail": DENIED })),
        "a workspace admin is not a project member"
    );
    assert!(!admin.has_rate_headers(), "a refused request is not counted");
    assert_eq!(call(&w, Who::Member, "POST", &base, &json!({"name": "Yes"})).status, 201);
    // and a project that is not there is refused to everyone
    let nowhere = format!("{}labels/", project_uri(0xBAD));
    for who in [Who::Admin, Who::Member] {
        assert_eq!(call(&w, who, "POST", &nowhere, &json!({"name": "x"})).status, 403, "{who:?}");
    }
}

// --- making a label -----------------------------------------------------------------------------------------------------------

/// The corpus's `create`: 201; the place is the greatest plus 10000 (85535 + 10000 = 95535, as the oracle's); the maker is the caller
/// and nobody has changed it yet; what the request did not say is empty.
#[test]
fn a_new_label_answers_201_and_goes_after_the_greatest_one_by_ten_thousand() {
    let w = real_world();
    let r = call(
        &w,
        Who::Admin,
        "POST",
        &labels(),
        &json!({"name": "oracle-scratch", "color": "#ff00ff", "sort_order": 1}),
    );
    assert_eq!(r.status, 201);
    assert!(r.has_rate_headers());
    assert_eq!(keys(&r.body), set(&LABEL_KEYS));
    let b = &r.body;
    assert_eq!(b["sort_order"], json!(95535.0), "the client's place is not kept");
    assert_eq!(
        (b["name"].clone(), b["color"].clone(), b["description"].clone()),
        (json!("oracle-scratch"), json!("#ff00ff"), json!(""))
    );
    assert_eq!(
        (b["created_by"].clone(), b["updated_by"].clone(), b["parent"].clone()),
        (json!(Who::Admin.id().0), Value::Null, Value::Null)
    );
    assert_eq!((b["project"].clone(), b["workspace"].clone()), (json!(id(PROJECT)), json!(id(WS))));
    // the next goes after that one, and the list is newest first
    let r2 = call(&w, Who::Member, "POST", &labels(), &json!({"name": "second"}));
    assert_eq!((r2.body["sort_order"].clone(), r2.body["color"].clone()), (json!(105_535.0), json!("")));
    assert_eq!(listed(&w), ["second", "oracle-scratch", "docs", "feature", "bug"]);
}

/// DRF's messages for a creation, field by field. The first comes from the corpus (`create-missing-name`).
#[rstest]
#[case::no_name(json!({"color": "#000000"}), json!({"name": ["This field is required."]}))]
#[case::nothing(json!({}), json!({"name": ["This field is required."]}))]
#[case::blank_name(json!({"name": "  "}), json!({"name": ["This field may not be blank."]}))]
#[case::null_name(json!({"name": null}), json!({"name": ["This field may not be null."]}))]
#[case::a_list_name(json!({"name": ["x"]}), json!({"name": ["Not a valid string."]}))]
#[case::a_long_name(json!({"name": "n".repeat(256)}), json!({"name": ["Ensure this field has no more than 255 characters."]}))]
#[case::a_null_colour(json!({"name": "n", "color": null}), json!({"color": ["This field may not be null."]}))]
#[case::a_long_colour(json!({"name": "n", "color": "c".repeat(256)}), json!({"color": ["Ensure this field has no more than 255 characters."]}))]
#[case::a_null_description(json!({"name": "n", "description": null}), json!({"description": ["This field may not be null."]}))]
#[case::a_long_source(json!({"name": "n", "external_source": "s".repeat(256)}), json!({"external_source": ["Ensure this field has no more than 255 characters."]}))]
#[case::a_long_id(json!({"name": "n", "external_id": "s".repeat(256)}), json!({"external_id": ["Ensure this field has no more than 255 characters."]}))]
#[case::not_a_number(json!({"name": "n", "sort_order": "x"}), json!({"sort_order": ["A valid number is required."]}))]
#[case::a_null_place(json!({"name": "n", "sort_order": null}), json!({"sort_order": ["This field may not be null."]}))]
#[case::a_parent_that_is_not_there(json!({"name": "n", "parent": "5a1b3c4d-0000-4000-8000-000000000001"}), json!({"parent": ["Invalid pk \"5a1b3c4d-0000-4000-8000-000000000001\" - object does not exist."]}))]
#[case::a_parent_quoted_as_spelled(json!({"name": "n", "parent": "{5A1B3C4D-0000-4000-8000-000000000001}"}), json!({"parent": ["Invalid pk \"{5A1B3C4D-0000-4000-8000-000000000001}\" - object does not exist."]}))]
#[case::a_parent_that_is_a_number(json!({"name": "n", "parent": 7}), json!({"parent": ["Invalid pk \"7\" - object does not exist."]}))]
#[case::a_parent_that_is_a_bool(json!({"name": "n", "parent": true}), json!({"parent": ["Incorrect type. Expected pk value, received bool."]}))]
#[case::several(json!({"color": null, "description": null, "sort_order": "y"}), json!({"name": ["This field is required."], "color": ["This field may not be null."], "description": ["This field may not be null."], "sort_order": ["A valid number is required."]}))]
fn a_creation_with_a_field_wrong_is_refused_with_drfs_message(#[case] body: Value, #[case] errors: Value) {
    let w = real_world();
    let r = call(&w, Who::Admin, "POST", &labels(), &body);
    assert_eq!((r.status, &r.body), (400, &errors));
    assert!(r.has_rate_headers());
    assert_eq!(listed(&w), ["docs", "feature", "bug"], "nothing was made");
}

/// A `parent` that the key column cannot read makes Django raise from inside the field: no DRF field catches it, so the answer is the
/// base view's bare 400, the fields before it are not reported, and the fields after it are not read.
#[rstest]
#[case::text(json!("nope"))]
#[case::too_short(json!("5a1b3c4d"))]
#[case::a_float(json!(1.5))]
#[case::a_negative_number(json!(-1))]
#[case::a_list(json!(["5a1b3c4d-0000-4000-8000-000000000001"]))]
#[case::an_object(json!({"id": "5a1b3c4d-0000-4000-8000-000000000001"}))]
fn a_parent_the_key_cannot_read_is_a_bare_400_that_hides_the_other_fields(#[case] parent: Value) {
    let w = real_world();
    for (method, uri) in [("POST", labels()), ("PATCH", label(bug()))] {
        // the name is missing on the creation and the place is wrong on both, and neither is reported
        let r = call(&w, Who::Admin, method, &uri, &json!({"parent": parent, "sort_order": "x"}));
        assert_eq!((r.status, r.body), (400, json!({"error": "Please provide valid detail"})), "{method}");
    }
    // a name the field before it refused is hidden too
    let r = call(&w, Who::Admin, "POST", &labels(), &json!({"name": null, "parent": parent}));
    assert_eq!(r.body, json!({"error": "Please provide valid detail"}));
    assert_eq!(listed(&w), ["docs", "feature", "bug"]);
}

/// The three answers to a name that is taken: a creation is a 409 naming the label that has it (the corpus's `create-duplicate`), a
/// name sent with spaces round it finds no row to name (the pin's view raises: a 500), and a change is the base view's 400 (the
/// corpus's `patch-rename-to-existing`).
#[test]
fn a_name_that_is_taken_gets_three_different_answers() {
    let w = real_world();
    let r = call(&w, Who::Admin, "POST", &labels(), &json!({"name": "bug"}));
    assert_eq!(
        (r.status, r.body),
        (409, json!({"error": "Label with the same name already exists in the project", "id": bug()}))
    );
    let padded = call(&w, Who::Admin, "POST", &labels(), &json!({"name": " bug "}));
    assert_eq!((padded.status, padded.body.clone()), (500, json!({"error": PLANE_500})));
    assert!(padded.has_rate_headers());
    // spaces round a name that is free are trimmed away
    let made = call(&w, Who::Admin, "POST", &labels(), &json!({"name": "  Padded  "}));
    assert_eq!((made.status, made.body["name"].clone()), (201, json!("Padded")));
    // another case is another name
    assert_eq!(call(&w, Who::Admin, "POST", &labels(), &json!({"name": "Bug"})).status, 201);
    // a change onto a name another label has
    let feature = id(LABEL_FEATURE).hyphenated().to_string();
    let r = call(&w, Who::Admin, "PATCH", &label(&feature), &json!({"name": "bug"}));
    assert_eq!((r.status, r.body), (400, json!({"error": "The payload is not valid"})));
    // and onto its own name, which is not another's
    assert_eq!(call(&w, Who::Admin, "PATCH", &label(&feature), &json!({"name": "feature"})).status, 200);
}

/// A key (`external_source` and `external_id`) is refused only when both halves are named, and the answer names the label that has it
/// on a creation and the label being changed on a change.
#[test]
fn an_integration_key_that_another_label_has_is_a_409() {
    let w = real_world();
    let first =
        call(&w, Who::Admin, "POST", &labels(), &json!({"name": "One", "external_source": "gh", "external_id": "1"}));
    assert_eq!((first.body["external_source"].clone(), first.body["external_id"].clone()), (json!("gh"), json!("1")));
    let taken =
        call(&w, Who::Admin, "POST", &labels(), &json!({"name": "Two", "external_source": "gh", "external_id": "1"}));
    assert_eq!(
        (taken.status, taken.body),
        (
            409,
            json!({"error": "Label with the same external id and external source already exists", "id": first.body["id"]})
        )
    );
    for body in [
        json!({"name": "Three", "external_source": "jira", "external_id": "1"}),
        json!({"name": "Four", "external_source": "gh", "external_id": "2"}),
        json!({"name": "Five", "external_id": "1"}),
        json!({"name": "Six", "external_source": "gh"}),
    ] {
        assert_eq!(call(&w, Who::Admin, "POST", &labels(), &body).status, 201, "{body}");
    }
    // a change to a key that another label has names the label being changed, not the one that has the key
    let feature = id(LABEL_FEATURE).hyphenated().to_string();
    let r = call(&w, Who::Admin, "PATCH", &label(&feature), &json!({"external_source": "gh", "external_id": "1"}));
    assert_eq!(
        (r.status, r.body),
        (409, json!({"error": "Label with the same external id and external source already exists", "id": feature}))
    );
    // the label that has the key keeps it, and a key half named is not checked
    let holder = first.body["id"].as_str().unwrap().to_string();
    assert_eq!(
        call(&w, Who::Admin, "PATCH", &label(&holder), &json!({"external_source": "gh", "external_id": "1"})).status,
        200
    );
    assert_eq!(call(&w, Who::Admin, "PATCH", &label(&feature), &json!({"external_id": "1"})).status, 200);
}

/// A body that is not an object: a list is DRF's "Expected a dictionary" 400; the other cases are the same as for projects.
#[test]
fn a_body_that_is_not_an_object_is_refused() {
    let w = real_world();
    let post = |ct: &str, body: &str| {
        send_raw(
            &w.router,
            "POST",
            &labels(),
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
    let empty = send_raw(&w.router, "POST", &labels(), &[("x-api-key", Who::Admin.key())], Vec::new());
    assert_eq!(empty.body, json!({"name": ["This field is required."]}));
}

// --- parents ------------------------------------------------------------------------------------------------------------------

/// A parent is a live label of the same project (D-032): the pin takes any label of any project, which would let a request name, and
/// learn of, labels it cannot see.
#[test]
fn a_parent_is_a_live_label_of_the_same_project() {
    let w = real_world();
    let other = project_of(&w, Who::Admin, "OTH");
    let foreign = call(&w, Who::Admin, "POST", &format!("{}labels/", project_uri(other)), &json!({"name": "foreign"}))
        .body["id"]
        .as_str()
        .unwrap()
        .to_string();
    let child = call(&w, Who::Admin, "POST", &labels(), &json!({"name": "child", "parent": bug()}));
    assert_eq!((child.status, child.body["parent"].clone()), (201, json!(bug())));
    // another project's label is not there to name: the answer is the one for a label that does not exist
    let r = call(&w, Who::Admin, "POST", &labels(), &json!({"name": "x", "parent": foreign}));
    assert_eq!(
        (r.status, r.body),
        (400, json!({"parent": [format!("Invalid pk \"{foreign}\" - object does not exist.")]}))
    );
    let child_id = child.body["id"].as_str().unwrap().to_string();
    let r = call(&w, Who::Admin, "PATCH", &label(&child_id), &json!({"parent": foreign}));
    assert_eq!(r.status, 400);
    // a change can clear the parent and can name the label itself, as the pin allows
    let cleared = call(&w, Who::Admin, "PATCH", &label(&child_id), &json!({"parent": null}));
    assert_eq!((cleared.status, cleared.body["parent"].clone()), (200, Value::Null));
    let empty_text = call(&w, Who::Admin, "PATCH", &label(&child_id), &json!({"parent": bug()}));
    assert_eq!(empty_text.body["parent"], json!(bug()));
    let cleared = call(&w, Who::Admin, "PATCH", &label(&child_id), &json!({"parent": ""}));
    assert_eq!(
        (cleared.status, cleared.body["parent"].clone()),
        (200, Value::Null),
        "an empty string is none, for a relation"
    );
    let itself = call(&w, Who::Admin, "PATCH", &label(&child_id), &json!({"parent": child_id}));
    assert_eq!((itself.status, itself.body["parent"].clone()), (200, json!(child_id)));
}

// --- changing a label ---------------------------------------------------------------------------------------------------------

/// The corpus's `patch`: only what the body names changes, and who changed it is recorded; the place can be set on a change.
#[test]
fn a_change_sets_what_it_names_and_records_who_changed_it() {
    let w = real_world();
    let r = call(&w, Who::Admin, "PATCH", &label(bug()), &json!({"color": "#00ff00"}));
    assert_eq!(r.status, 200);
    assert!(r.has_rate_headers());
    assert_eq!(keys(&r.body), set(&LABEL_KEYS));
    assert_eq!((r.body["color"].clone(), r.body["name"].clone()), (json!("#00ff00"), json!("bug")));
    assert_eq!(r.body["sort_order"], json!(65535.0), "kept");
    assert_eq!((r.body["updated_by"].clone(), r.body["created_by"].clone()), (json!(Who::Admin.id().0), Value::Null));
    // the name, the description, the place and a key can be set, and a key cleared
    let r = call(
        &w,
        Who::Member,
        "PATCH",
        &label(bug()),
        &json!({"name": "defect", "description": "d", "sort_order": "123.5", "external_source": "gh", "external_id": "9"}),
    );
    assert_eq!(
        (r.body["name"].clone(), r.body["description"].clone(), r.body["sort_order"].clone()),
        (json!("defect"), json!("d"), json!(123.5))
    );
    assert_eq!(
        (r.body["external_source"].clone(), r.body["external_id"].clone(), r.body["updated_by"].clone()),
        (json!("gh"), json!("9"), json!(Who::Member.id().0))
    );
    let r = call(&w, Who::Admin, "PATCH", &label(bug()), &json!({"external_source": null, "external_id": null}));
    assert_eq!((r.body["external_source"].clone(), r.body["external_id"].clone()), (Value::Null, Value::Null));
    // naming nothing is a change that changes nothing but who and when
    assert_eq!(call(&w, Who::Admin, "PATCH", &label(bug()), &json!({})).status, 200);
    // the list shows the change, at its new place in the order of creation, which a change of place does not move
    assert_eq!(listed(&w), ["docs", "feature", "defect"]);
}

/// The base view parses the body before the handler runs, so a body that cannot be parsed is refused whether or not the label is
/// there; the handler then finds the label, and only then does the serializer read what the parsed body says.
#[test]
fn a_change_parses_the_body_then_finds_its_label_then_reads_what_it_says() {
    let w = real_world();
    let nowhere = label(id(0xBAD).hyphenated());
    let patch = |uri: &str, ct: &str, body: &str| {
        send_raw(
            &w.router,
            "PATCH",
            uri,
            &[("x-api-key", Who::Admin.key()), ("content-type", ct)],
            body.as_bytes().to_vec(),
        )
    };
    for uri in [&nowhere, &label(bug())] {
        assert_eq!(patch(uri, "text/plain", "x").status, 415, "{uri}");
        let bad = patch(uri, "application/json", "{bad");
        assert_eq!(bad.status, 400, "{uri}");
        assert!(bad.body["detail"].as_str().unwrap().starts_with("JSON parse error - "), "{uri}");
        let scalar = patch(uri, "application/json", "\"s\"");
        assert_eq!(scalar.body, json!({"detail": "Request body must be a JSON object or array."}), "{uri}");
    }
    // a list parses, so the label is looked for before the serializer refuses it
    let r = patch(&nowhere, "application/json", "[1]");
    assert_eq!((r.status, r.body), (404, json!({ "error": NOT_FOUND })));
    let r = patch(&label(bug()), "application/json", "[1]");
    assert_eq!(r.body, json!({"non_field_errors": ["Invalid data. Expected a dictionary, but got list."]}));
    // so is a field that is wrong, and an empty body is no data at all
    let r = patch(&nowhere, "application/json", "{\"sort_order\": \"x\"}");
    assert_eq!((r.status, r.body), (404, json!({ "error": NOT_FOUND })));
    assert_eq!(patch(&nowhere, "text/plain", "").status, 404);
    let r = call(&w, Who::Admin, "PATCH", &label(bug()), &json!({"sort_order": "x"}));
    assert_eq!((r.status, r.body), (400, json!({"sort_order": ["A valid number is required."]})));
}

// --- deleting a label ---------------------------------------------------------------------------------------------------------

/// The corpus's `delete`, `detail-after-delete` and `delete-again`: 204 with no body, then it is gone, and a second deletion is a 404.
#[test]
fn a_label_is_deleted_and_then_it_is_gone_and_its_name_is_free() {
    let w = real_world();
    let made = call(&w, Who::Admin, "POST", &labels(), &json!({"name": "scratch"}));
    let scratch = made.body["id"].as_str().unwrap().to_string();
    let r = call(&w, Who::Admin, "DELETE", &label(&scratch), &json!({}));
    assert_eq!((r.status, r.body.clone()), (204, Value::Null));
    assert!(r.has_rate_headers());
    let gone = get(&w, Who::Admin, &label(&scratch));
    assert_eq!((gone.status, gone.body), (404, json!({ "error": NOT_FOUND })));
    let again = call(&w, Who::Admin, "DELETE", &label(&scratch), &json!({}));
    assert_eq!((again.status, again.body), (404, json!({ "error": NOT_FOUND })));
    assert_eq!(listed(&w), ["docs", "feature", "bug"]);
    // its name is free, and the place it had is the next
    let again = call(&w, Who::Admin, "POST", &labels(), &json!({"name": "scratch"}));
    assert_eq!((again.status, again.body["sort_order"].clone()), (201, json!(95535.0)));
    // a label that is nowhere is not there to delete
    assert_eq!(call(&w, Who::Admin, "DELETE", &label(id(0xBAD).hyphenated()), &json!({})).status, 404);
}

/// A deletion takes the labels under the label (and theirs in turn), and the work items lose every one of them.
#[test]
fn a_deletion_takes_the_labels_under_it_and_the_work_items_hold_on_them() {
    let w = real_world();
    let child = call(&w, Who::Admin, "POST", &labels(), &json!({"name": "child", "parent": bug()})).body["id"]
        .as_str()
        .unwrap()
        .to_string();
    let grandchild = call(&w, Who::Admin, "POST", &labels(), &json!({"name": "grandchild", "parent": child})).body
        ["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (feature, grandchild_id) = (id(LABEL_FEATURE), grandchild.parse::<Uuid>().unwrap());
    let mut held = vec![grandchild_id, feature];
    held.sort();
    let item = item_with(&w, &held);
    assert_eq!(labels_of(&w, item), held);

    assert_eq!(call(&w, Who::Admin, "DELETE", &label(bug()), &json!({})).status, 204);
    assert_eq!(listed(&w), ["docs", "feature"], "the child and the grandchild went with it");
    assert_eq!(get(&w, Who::Admin, &label(&grandchild)).status, 404);
    assert_eq!(labels_of(&w, item), [feature]);
}
