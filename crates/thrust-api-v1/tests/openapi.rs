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
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/oracle-v1/schema/described.operations");
    let text = std::fs::read_to_string(path).expect("the list of described operations");
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).count()
}

fn project_id() -> String {
    Uuid::from_u128(PROJECT).to_string()
}

fn state_id() -> String {
    Uuid::from_u128(STATE_TODO).to_string()
}

/// A path of the document with the seeded world's values in its placeholders. A placeholder this does not know is a defect of the
/// test, not a path to skip.
fn filled(path: &str) -> String {
    let uuid = project_id();
    let mut out = path.replace("{slug}", SLUG).replace("{pk}", &uuid).replace("{project_id}", &uuid);
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
