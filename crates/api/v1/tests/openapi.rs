//! The OpenAPI document of the crate (the `openapi` feature, issue #58) against the crate's own router and the bodies it answers with.
//!
//! The document is written from types the routes serialize, so a body cannot say one thing and its schema another except through an
//! attribute that one side reads and the other does not (`skip_serializing_if`, `schema(...)`). Its paths are written beside the
//! handlers but are not the router's: a path string can say one thing and `.route(...)` another. These tests close both:
//!
//! * every operation the document describes is a route of the router (and a path the router does not have is the URLconf's 404,
//!   so the test can tell the two apart);
//! * the keys of a real body are the properties its schema names, and a key that is `null` in a real body is one its schema lets be.
#![cfg(feature = "openapi")]
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
use serde_json::{json, Value};
use thrust_contracts::Uuid;

const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];

fn document() -> Value {
    serde_json::to_value(thrust_api_v1::openapi::openapi()).expect("the document is JSON")
}

/// Every (METHOD, path) the document declares.
fn operations(doc: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (path, item) in doc["paths"].as_object().expect("paths") {
        for m in METHODS {
            if item.get(m).is_some() {
                out.push((m.to_uppercase(), path.clone()));
            }
        }
    }
    out
}

/// The operations the committed list says the document describes: the size of what the test below walks, so that it cannot shrink
/// to nothing and pass.
fn listed_operations() -> usize {
    let path = concat!(env!("THRUST_ROOT"), "/tools/oracle-v1/schema/described.operations");
    let text = std::fs::read_to_string(path).expect("the list of described operations");
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).count()
}

fn project_id() -> String {
    Uuid::from_u128(PROJECT).to_string()
}

fn state_id() -> String {
    Uuid::from_u128(STATE_TODO).to_string()
}

fn label_id() -> String {
    Uuid::from_u128(LABEL_BUG).to_string()
}

/// A path of the document with the seeded world's values in its placeholders. A placeholder this does not know is a defect of the
/// test, not a path to skip.
fn filled(path: &str) -> String {
    let uuid = project_id();
    // the lookup by `KEY-n`: any key and number make the route, which the router then answers (as a refusal, here)
    let mut out = path
        .replace("{slug}", SLUG)
        .replace("{pk}", &uuid)
        .replace("{project_id}", &uuid)
        .replace("{project_identifier}", "KEY")
        .replace("{issue_identifier}", "1");
    for other in ["{state_id}", "{label_id}", "{issue_id}"] {
        out = out.replace(other, &uuid);
    }
    assert!(!out.contains('{'), "a placeholder this test cannot fill: {path}");
    out
}

fn is_page_not_found(r: &Reply) -> bool {
    r.status == 404 && r.body == json!({ "error": "Page not found." })
}

#[test]
fn every_operation_the_document_describes_is_a_route_of_the_router() {
    let doc = document();
    let ops = operations(&doc);
    assert_eq!(
        ops.len(),
        listed_operations(),
        "the document describes the operations tools/oracle-v1/schema/described.operations lists"
    );
    for (method, path) in &ops {
        // a world of its own: a DELETE must not take the project from the requests after it
        let w = real_world();
        let r = send_as(&w.router, "key-admin", method, &filled(path));
        assert!(!is_page_not_found(&r), "{method} {path}: the router has no such route (the URLconf's 404)");
        assert_ne!(r.status, 405, "{method} {path}: the route is there and does not take that method");
    }
}

/// The control of the test above: it can tell a route from no route. Without this it would pass if the router answered everything.
#[test]
fn a_path_the_router_does_not_have_is_the_urlconfs_404_and_a_method_it_does_not_take_is_a_405() {
    let w = real_world();
    let missing = format!("/api/v1/workspaces/{SLUG}/projects/{}/not-a-route/", project_id());
    assert!(is_page_not_found(&send_as(&w.router, "key-admin", "GET", &missing)));
    // the project's own path with a method it has no handler for
    let uri = format!("/api/v1/workspaces/{SLUG}/projects/{}/archive/", project_id());
    assert_eq!(send_as(&w.router, "key-admin", "GET", &uri).status, 405);
}

/// The properties of a component schema, and which of them are required.
fn schema_of(doc: &Value, name: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let schema = &doc["components"]["schemas"][name];
    let keys = |v: &Value| v.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    let required =
        schema["required"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
    assert!(schema["properties"].is_object(), "the document has no schema named {name}");
    (keys(&schema["properties"]), required.unwrap_or_default())
}

/// Whether a schema lets a value be `null`: its `type` names it, or it names no type at all (any value).
fn lets_be_null(schema: &Value) -> bool {
    match &schema["type"] {
        Value::Array(types) => types.iter().any(|t| t == "null"),
        Value::Null => true,
        _ => false,
    }
}

/// A body's keys are the schema's properties (the schema may list more, the optional ones a body leaves out), its required ones are all
/// there, and a `null` is where the schema allows one.
fn assert_body_is_its_schema(doc: &Value, schema_name: &str, body: &Value, optional_and_absent: &[&str], what: &str) {
    let (props, required) = schema_of(doc, schema_name);
    let keys: BTreeSet<String> =
        body.as_object().unwrap_or_else(|| panic!("{what}: not an object")).keys().cloned().collect();
    let unnamed: Vec<_> = keys.difference(&props).collect();
    assert!(unnamed.is_empty(), "{what}: keys the schema {schema_name} does not name: {unnamed:?}");
    let absent: BTreeSet<String> = props.difference(&keys).cloned().collect();
    let expected_absent: BTreeSet<String> = optional_and_absent.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(absent, expected_absent, "{what}: the properties of {schema_name} that the body leaves out");
    let missing: Vec<_> = required.difference(&keys).collect();
    assert!(missing.is_empty(), "{what}: required properties of {schema_name} that the body lacks: {missing:?}");
    for (k, v) in body.as_object().expect("an object") {
        if v.is_null() {
            let property = &doc["components"]["schemas"][schema_name]["properties"][k];
            assert!(
                lets_be_null(property),
                "{what}: `{k}` is null in the body and {schema_name} does not allow it: {property}"
            );
        }
    }
}

fn get(w: &World, uri: &str) -> Value {
    let r = send_as(&w.router, "key-admin", "GET", uri);
    assert_eq!(r.status, 200, "{uri}: {:?}", r.body);
    r.body
}

#[test]
fn a_projects_body_is_its_schema() {
    let doc = document();
    let w = real_world();
    let base = format!("/api/v1/workspaces/{SLUG}/projects");
    // one project: no `sort_order` (the key is the list's)
    let one = get(&w, &format!("{base}/{}/", project_id()));
    assert_body_is_its_schema(&doc, "ProjectDto", &one, &["sort_order"], "GET project");
    // a list: 44 keys
    let list = get(&w, &format!("{base}/"));
    assert_body_is_its_schema(&doc, "Page_ProjectDto", &list, &[], "GET projects (the envelope)");
    assert_body_is_its_schema(&doc, "ProjectDto", &list["results"][0], &[], "GET projects (a row)");
    // the lite list
    let lite = get(&w, &format!("/api/v1/workspaces/{SLUG}/projects-lite/"));
    assert_body_is_its_schema(&doc, "Page_ProjectLiteDto", &lite, &[], "GET projects-lite (the envelope)");
    assert_body_is_its_schema(&doc, "ProjectLiteDto", &lite["results"][0], &[], "GET projects-lite (a row)");
}

/// The control of the test above: it can tell a body from a schema that does not describe it.
#[test]
fn a_body_that_the_schema_does_not_describe_is_refused() {
    let doc = document();
    let w = real_world();
    let one = get(&w, &format!("/api/v1/workspaces/{SLUG}/projects/{}/", project_id()));
    let refused = |body: &Value, what: &str| {
        let r = std::panic::catch_unwind(|| assert_body_is_its_schema(&doc, "ProjectDto", body, &["sort_order"], what));
        assert!(r.is_err(), "{what}: a body the schema does not describe was accepted");
    };
    let mut extra = one.clone();
    extra["a_key_nobody_declared"] = json!(1);
    refused(&extra, "an extra key");
    let mut short = one.clone();
    short.as_object_mut().expect("an object").remove("name");
    refused(&short, "a missing required key");
    let mut hole = one.clone();
    hole["is_member"] = Value::Null;
    refused(&hole, "a null where the schema allows none");
}

/// The states of the seeded project: the default, one more, and the intake state, which a list leaves out. All 18 keys are in every
/// row, `null` or not.
#[test]
fn a_states_body_is_its_schema() {
    let doc = document();
    let w = real_world();
    let base = format!("/api/v1/workspaces/{SLUG}/projects/{}/states", project_id());
    let one = get(&w, &format!("{base}/{}/", state_id()));
    assert_body_is_its_schema(&doc, "StateDto", &one, &[], "GET state");
    let list = get(&w, &format!("{base}/"));
    assert_body_is_its_schema(&doc, "Page_StateDto", &list, &[], "GET states (the envelope)");
    let rows = list["results"].as_array().expect("results");
    assert_eq!(rows.len(), 2, "the default state and one more: the intake state is not listed");
    for row in rows {
        assert_body_is_its_schema(&doc, "StateDto", row, &[], "GET states (a row)");
    }
}

/// The control of the test above, for the States' schema: an extra key, a lost key and a `null` that the schema does not allow.
#[test]
fn a_state_body_that_the_schema_does_not_describe_is_refused() {
    let doc = document();
    let w = real_world();
    let one = get(&w, &format!("/api/v1/workspaces/{SLUG}/projects/{}/states/{}/", project_id(), state_id()));
    let refused = |body: &Value, what: &str| {
        let r = std::panic::catch_unwind(|| assert_body_is_its_schema(&doc, "StateDto", body, &[], what));
        assert!(r.is_err(), "{what}: a body the schema does not describe was accepted");
    };
    let mut extra = one.clone();
    extra["a_key_nobody_declared"] = json!(1);
    refused(&extra, "an extra key");
    let mut short = one.clone();
    short.as_object_mut().expect("an object").remove("color");
    refused(&short, "a missing key");
    let mut hole = one.clone();
    hole["group"] = Value::Null;
    refused(&hole, "a null where the schema allows none");
}

/// The labels of the seeded project (`bug`, `feature`, `docs`), and one made with a parent and the keys an integration gives it, so that
/// each key is checked where it is `null` and where it is not. All 15 keys are in every row.
#[test]
fn a_labels_body_is_its_schema() {
    let doc = document();
    let w = real_world();
    let base = format!("/api/v1/workspaces/{SLUG}/projects/{}/labels", project_id());
    let one = get(&w, &format!("{base}/{}/", label_id()));
    assert_body_is_its_schema(&doc, "LabelDto", &one, &[], "GET label");
    let list = get(&w, &format!("{base}/"));
    assert_body_is_its_schema(&doc, "Page_LabelDto", &list, &[], "GET labels (the envelope)");
    let rows = list["results"].as_array().expect("results");
    assert_eq!(rows.len(), 3, "bug, feature and docs");
    for row in rows {
        assert_body_is_its_schema(&doc, "LabelDto", row, &[], "GET labels (a row)");
    }
    let child = json!({"name": "child", "parent": label_id(), "external_source": "gh", "external_id": "7"});
    let made = send_raw(
        &w.router,
        "POST",
        &format!("{base}/"),
        &[("x-api-key", "key-admin"), ("content-type", "application/json")],
        child.to_string().into_bytes(),
    );
    assert_eq!(made.status, 201, "{:?}", made.body);
    assert_body_is_its_schema(&doc, "LabelDto", &made.body, &[], "POST label (a child)");
    assert_eq!(made.body["parent"], json!(label_id()), "the case ran with a parent that is not `null`");
    assert_eq!((made.body["external_source"].clone(), made.body["external_id"].clone()), (json!("gh"), json!("7")));
}

/// The control of the test above, for the Labels' schema: an extra key, a lost key, and a `null` where the schema allows none (`name`,
/// and `project`, which the pin's schema lets be `null` and this document does not, because a label always has one).
#[test]
fn a_label_body_that_the_schema_does_not_describe_is_refused() {
    let doc = document();
    let w = real_world();
    let one = get(&w, &format!("/api/v1/workspaces/{SLUG}/projects/{}/labels/{}/", project_id(), label_id()));
    let refused = |body: &Value, what: &str| {
        let r = std::panic::catch_unwind(|| assert_body_is_its_schema(&doc, "LabelDto", body, &[], what));
        assert!(r.is_err(), "{what}: a body the schema does not describe was accepted");
    };
    let mut extra = one.clone();
    extra["a_key_nobody_declared"] = json!(1);
    refused(&extra, "an extra key");
    let mut short = one.clone();
    short.as_object_mut().expect("an object").remove("color");
    refused(&short, "a missing key");
    for key in ["name", "project"] {
        let mut hole = one.clone();
        hole[key] = Value::Null;
        refused(&hole, &format!("a null `{key}` where the schema allows none"));
    }
}

/// A work item with everything the body can hold (a state, a priority, two dates, a point, a person, a label, an integration key) and
/// one with nothing, read the ways a read can take: as stored, with `expand`, by id in either family, by the identifier of its project,
/// in a list, as the one work item a list answers for an integration key, and in the search.
#[test]
fn a_work_items_body_is_its_schema() {
    let doc = document();
    let w = real_world();
    let base = format!("/api/v1/workspaces/{SLUG}/projects/{}", project_id());
    let person = Who::Member.id().0;
    let full = json!({
        "name": "full", "description_html": "<p>x</p>", "priority": "high", "start_date": "2026-01-01", "target_date": "2026-02-01",
        "point": 3, "state": state_id(), "assignees": [person], "labels": [label_id()],
        "external_source": "gh", "external_id": "7",
    });
    let make = |body: &Value| {
        let r = send_json(&w.router, "key-admin", "POST", &format!("{base}/issues/"), body);
        assert_eq!(r.status, 201, "{:?}", r.body);
        r.body
    };
    let made = make(&full);
    assert_body_is_its_schema(&doc, "IssueDto", &made, &[], "POST a work item that has everything");
    assert_eq!(made["state"], json!(state_id()), "the case ran with a state that is not `null`");
    let bare = make(&json!({"name": "bare"}));
    assert_body_is_its_schema(&doc, "IssueDto", &bare, &[], "POST a work item that has nothing");
    let id = made["id"].as_str().expect("an id").to_string();
    for family in ["issues", "work-items"] {
        let one = get(&w, &format!("{base}/{family}/{id}/"));
        assert_body_is_its_schema(&doc, "IssueDto", &one, &[], &format!("GET {family}/id"));
        let list = get(&w, &format!("{base}/{family}/"));
        assert_body_is_its_schema(&doc, "Page_IssueDto", &list, &[], &format!("GET {family}/ (the envelope)"));
        let rows = list["results"].as_array().expect("results");
        assert_eq!(rows.len(), 2, "the two work items");
        for row in rows {
            assert_body_is_its_schema(&doc, "IssueDto", row, &[], &format!("GET {family}/ (a row)"));
        }
        // with both halves of a key the list is the one work item, not a page
        let by_key = get(&w, &format!("{base}/{family}/?external_id=7&external_source=gh"));
        assert_body_is_its_schema(&doc, "IssueDto", &by_key, &[], &format!("GET {family}/ by a key"));
        let ident = get(&w, &format!("{base}/"))["identifier"].as_str().expect("an identifier").to_string();
        let by_ident = get(&w, &format!("/api/v1/workspaces/{SLUG}/{family}/{ident}-1/"));
        assert_body_is_its_schema(&doc, "IssueDto", &by_ident, &[], &format!("GET {family}/KEY-n"));
        let found = get(&w, &format!("/api/v1/workspaces/{SLUG}/{family}/search/?search=full"));
        assert_body_is_its_schema(&doc, "IssueSearchDto", &found, &[], &format!("GET {family}/search/"));
        let hit = &found["issues"][0];
        assert_body_is_its_schema(&doc, "IssueSearchItemDto", hit, &[], &format!("GET {family}/search/ (a hit)"));
        assert!(
            hit["sequence_id"].is_i64(),
            "{family}: a hit's `sequence_id` is a whole number (the corpus), not the pin's string"
        );
    }
    // not expanded, the state, the person and the label are ids; expanded, each is its object, and the object is its schema
    assert!(made["assignees"][0].is_string() && made["labels"][0].is_string(), "ids when `expand` is not asked");
    let expanded = get(&w, &format!("{base}/issues/{id}/?expand=state,assignees,labels"));
    assert_body_is_its_schema(&doc, "IssueDto", &expanded, &[], "GET with expand");
    assert_body_is_its_schema(&doc, "StateBriefDto", &expanded["state"], &[], "expanded state");
    assert_body_is_its_schema(&doc, "UserBriefDto", &expanded["assignees"][0], &[], "expanded person");
    assert_body_is_its_schema(&doc, "LabelDto", &expanded["labels"][0], &[], "expanded label");
    // the list says it is a page or one work item, and the page is the schema the other lists have
    let listing = doc["components"]["schemas"]["Listing_IssueDto"]["oneOf"].as_array().expect("a oneOf");
    assert_eq!(listing.len(), 2, "a list answers a page or one work item");
    assert_eq!(listing[0]["$ref"], json!("#/components/schemas/Page_IssueDto"), "the page");
    // utoipa writes the parameter of a generic type in place and not as a `$ref`; what it writes is the schema of an `IssueDto`
    assert_eq!(listing[1], doc["components"]["schemas"]["IssueDto"], "the one work item that a list can answer");
}

/// The control of the test above, for the work items' schemas: an extra key, a lost key, a `null` where the schema allows none (the
/// name, the project, the priority), and an expanded state that has lost a key.
#[test]
fn a_work_item_body_that_the_schema_does_not_describe_is_refused() {
    let doc = document();
    let w = real_world();
    let base = format!("/api/v1/workspaces/{SLUG}/projects/{}", project_id());
    let made = send_json(
        &w.router,
        "key-admin",
        "POST",
        &format!("{base}/issues/"),
        &json!({"name": "a", "state": state_id()}),
    );
    assert_eq!(made.status, 201, "{:?}", made.body);
    let id = made.body["id"].as_str().expect("an id").to_string();
    let expanded = get(&w, &format!("{base}/issues/{id}/?expand=state"));
    let refused = |schema: &str, body: &Value, what: &str| {
        let r = std::panic::catch_unwind(|| assert_body_is_its_schema(&doc, schema, body, &[], what));
        assert!(r.is_err(), "{what}: a body the schema does not describe was accepted");
    };
    let mut extra = made.body.clone();
    extra["a_key_nobody_declared"] = json!(1);
    refused("IssueDto", &extra, "an extra key");
    let mut short = made.body.clone();
    short.as_object_mut().expect("an object").remove("labels");
    refused("IssueDto", &short, "a missing key");
    // `priority` is left out on purpose: the document writes it as `allOf` of the enum, which names no type, so a `null` is not a
    // refusal by type; the enum is tied to the route by the test below
    for key in ["name", "project", "is_draft"] {
        let mut hole = made.body.clone();
        hole[key] = Value::Null;
        refused("IssueDto", &hole, &format!("a null `{key}` where the schema allows none"));
    }
    let mut state = expanded["state"].clone();
    state.as_object_mut().expect("an object").remove("color");
    refused("StateBriefDto", &state, "an expanded state that has lost a key");
}

/// The pin's `PriorityEnum` is these five, in this order (an example the pin names, `openapi.v1.json`). The route takes each of them
/// and answers it, and takes no other, so the enum the document lists is the one the parser has.
#[test]
fn each_priority_the_document_lists_is_taken_and_answered_and_no_other_is() {
    let doc = document();
    let listed: Vec<&str> = doc["components"]["schemas"]["PriorityEnum"]["enum"]
        .as_array()
        .expect("an enum")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(listed, ["urgent", "high", "medium", "low", "none"]);
    let w = real_world();
    let base = format!("/api/v1/workspaces/{SLUG}/projects/{}/issues", project_id());
    let made = send_json(&w.router, "key-admin", "POST", &format!("{base}/"), &json!({"name": "a"}));
    let uri = format!("{base}/{}/", made.body["id"].as_str().expect("an id"));
    for priority in &listed {
        let r = send_json(&w.router, "key-admin", "PATCH", &uri, &json!({"priority": priority}));
        assert_eq!((r.status, r.body["priority"].clone()), (200, json!(priority)), "{priority}");
    }
    let refused = send_json(&w.router, "key-admin", "PATCH", &uri, &json!({"priority": "critical"}));
    assert_eq!(refused.status, 400, "{:?}", refused.body);
}

/// The pin documents each work item operation under `issues/` and again under `work-items/`, with an operation id that ends in `_2`.
/// The routes share their handlers, and the document derives the second family from the first: the same operation, apart from its id.
#[test]
fn the_two_families_of_paths_are_one_set_of_operations() {
    let doc = document();
    let paths = doc["paths"].as_object().expect("paths");
    let mut pairs = 0;
    for (path, item) in paths {
        if !path.contains("/issues/") {
            continue;
        }
        let twin = path.replacen("/issues/", "/work-items/", 1);
        let other = paths.get(&twin).unwrap_or_else(|| panic!("{path} has no twin under work-items/"));
        for method in METHODS {
            let (first, second) = (item.get(method), other.get(method));
            assert_eq!(first.is_some(), second.is_some(), "{method} {path}: only one family has it");
            let (Some(first), Some(second)) = (first, second) else { continue };
            let id = first["operationId"].as_str().expect("an operation id");
            assert!(!id.ends_with("_2"), "{id}: the first family's ids do not end in _2");
            assert_eq!(second["operationId"], json!(format!("{id}_2")), "{method} {path}");
            let mut same = second.clone();
            same["operationId"] = first["operationId"].clone();
            assert_eq!(first, &same, "{method} {path}: the two families differ");
            pairs += 1;
        }
    }
    assert_eq!(
        pairs, 7,
        "seven operations in each family: the list, create, retrieve, update, delete, the search and `KEY-n`"
    );
    let twos = operations(&doc).iter().filter(|(_, p)| p.contains("/work-items/")).count();
    assert_eq!(twos, pairs, "every operation under work-items/ is the twin of one under issues/");
}

/// A body that may be left out is an object, and an empty one is an empty object; the JSON value `null` is refused. The document says
/// the first and so does not describe a request body as `oneOf [T, null]` (utoipa's way of writing `Option<T>`, which `OptionalBodies`
/// takes out), and the routes answer the second.
#[test]
fn a_body_that_may_be_left_out_is_not_described_as_null_and_a_null_body_is_refused() {
    let doc = document();
    let mut bodies = 0;
    for (path, item) in doc["paths"].as_object().expect("paths") {
        for method in METHODS {
            let Some(content) = item.get(method).and_then(|op| op.get("requestBody")).and_then(|b| b.get("content"))
            else {
                continue;
            };
            for (media, body) in content.as_object().expect("content") {
                let schema = &body["schema"];
                let nulls = schema["oneOf"].as_array().is_some_and(|a| a.iter().any(|s| s["type"] == json!("null")));
                assert!(!nulls, "{method} {path} ({media}): a body described as `oneOf [.., null]`: {schema}");
                bodies += 1;
            }
        }
    }
    assert!(bodies >= 7, "the document has request bodies to check ({bodies})");
    let w = real_world();
    let base = format!("/api/v1/workspaces/{SLUG}/projects/{}", project_id());
    for uri in [
        format!("{base}/"),
        format!("{base}/states/{}/", state_id()),
        format!("{base}/labels/{}/", label_id()),
        format!("{base}/issues/{}/", project_id()),
    ] {
        let r = send_raw(
            &w.router,
            "PATCH",
            &uri,
            &[("x-api-key", "key-admin"), ("content-type", "application/json")],
            b"null".to_vec(),
        );
        assert_eq!(r.status, 400, "PATCH {uri}: {:?}", r.body);
        assert_eq!(r.body, json!({"detail": "Request body must be a JSON object or array."}), "PATCH {uri}");
    }
}

/// The request descriptions of a change are written by hand beside those of a creation, so that nothing else ties them: a change takes
/// the keys of a creation, with the same bounds, and requires none.
#[test]
fn a_change_takes_the_keys_of_a_creation_and_requires_none() {
    let doc = document();
    for (create, change) in [
        ("StateRequest", "PatchedStateRequest"),
        ("LabelCreateUpdateRequest", "PatchedLabelCreateUpdateRequest"),
        ("IssueRequest", "PatchedIssueRequest"),
    ] {
        let (c, p) = (&doc["components"]["schemas"][create], &doc["components"]["schemas"][change]);
        assert!(
            c["properties"].is_object() && p["properties"].is_object(),
            "{create} and {change} are in the document"
        );
        assert_eq!(c["properties"], p["properties"], "{change} takes the keys of {create}, with the same bounds");
        assert!(c["required"].as_array().is_some_and(|r| !r.is_empty()), "{create} requires something");
        assert!(p["required"].as_array().is_none_or(Vec::is_empty), "{change} requires nothing");
    }
}

/// D-031 (2): a name with no letter or digit that NFKD leaves in ASCII has the empty slug, on the pin and here, and the pin's schema
/// gives `slug` a pattern that forbids it. The document does not repeat that pattern (SD-012 of the register) because the route can
/// send an empty slug, which is what this shows; a pattern that rejects "" would be a claim the route does not keep.
#[test]
fn a_name_with_no_letter_has_an_empty_slug_which_the_document_does_not_forbid() {
    let doc = document();
    let w = real_world();
    let uri = format!("/api/v1/workspaces/{SLUG}/projects/{}/states/", project_id());
    let made = send_raw(
        &w.router,
        "POST",
        &uri,
        &[("x-api-key", "key-admin"), ("content-type", "application/json")],
        r##"{"name": "日本語", "color": "#fff"}"##.as_bytes().to_vec(),
    );
    assert_eq!(made.status, 200, "{:?}", made.body);
    assert_eq!(made.body["slug"], json!(""), "the route sends the empty slug of a name of CJK alone");
    let slug = &doc["components"]["schemas"]["StateDto"]["properties"]["slug"];
    assert!(slug.get("pattern").is_none(), "StateDto.slug claims a pattern that the route does not keep: {slug}");
}
