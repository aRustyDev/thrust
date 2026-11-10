//! The `Allow` header (issue #58): the pin's view puts its methods on EVERY answer it makes, an error and a refusal included, as
//! DRF does (`GET, POST`, comma and space), and an answer that no view made (the URLconf's 404) has none. Every case below is the
//! pinned Plane's own, from the oracle's corpus (`projects`, `states`, `labels`, `work_items`, `errors`) or measured on a running
//! one (a keyless 401), which is what the corpus comparison checks over the whole of it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use common::*;
use rstest::rstest;
use serde_json::json;
use thrust_contracts::Uuid;

fn ws() -> String {
    format!("/api/v1/workspaces/{SLUG}")
}

fn project() -> String {
    format!("{}/projects/{}", ws(), Uuid::from_u128(PROJECT))
}

fn nobody() -> Uuid {
    Uuid::from_u128(0xDEAD)
}

/// The identifier the cast project has (the world gives it none of its own): the corpus's `ORA-9999` is a number that is not there in a
/// project that is.
fn key_of_the_project() -> String {
    thrust_contracts::default_project_identifier(thrust_contracts::ProjectId(Uuid::from_u128(PROJECT)))
}

#[rstest]
// the lists, a creation that is refused (400) and one that is forbidden (403), and a call with no key at all (401)
#[case::projects_list("GET", format!("{}/projects/", ws()), "key-admin", 200, Some("GET, POST"))]
#[case::projects_create_refused("POST", format!("{}/projects/", ws()), "key-admin", 400, Some("GET, POST"))]
#[case::projects_create_forbidden("POST", format!("{}/projects/", ws()), "key-guest", 403, Some("GET, POST"))]
#[case::projects_no_key("GET", format!("{}/projects/", ws()), "", 401, Some("GET, POST"))]
#[case::projects_lite("GET", format!("{}/projects-lite/", ws()), "key-admin", 200, Some("GET"))]
#[case::a_project("GET", format!("{}/", project()), "key-admin", 200, Some("GET, PATCH, DELETE"))]
#[case::a_project_that_is_not_there("GET", format!("{}/projects/{}/", ws(), nobody()), "key-admin", 404, Some("GET, PATCH, DELETE"))]
#[case::the_archive("POST", format!("{}/archive/", project()), "key-guest", 403, Some("POST, DELETE"))]
#[case::states_list("GET", format!("{}/states/", project()), "key-admin", 200, Some("GET, POST"))]
#[case::a_state_that_is_not_there("GET", format!("{}/states/{}/", project(), nobody()), "key-admin", 404, Some("GET, PATCH, DELETE"))]
#[case::labels_list("GET", format!("{}/labels/", project()), "key-admin", 200, Some("GET, POST"))]
#[case::a_label_that_is_not_there("GET", format!("{}/labels/{}/", project(), nobody()), "key-admin", 404, Some("GET, PATCH, DELETE"))]
#[case::work_items_list("GET", format!("{}/work-items/", project()), "key-admin", 200, Some("GET, POST"))]
#[case::issues_list("GET", format!("{}/issues/", project()), "key-admin", 200, Some("GET, POST"))]
#[case::work_items_create_forbidden("POST", format!("{}/work-items/", project()), "key-guest", 403, Some("GET, POST"))]
#[case::a_work_item_that_is_not_there("GET", format!("{}/work-items/{}/", project(), nobody()), "key-admin", 404, Some("GET, PATCH, DELETE"))]
#[case::the_search("GET", format!("{}/work-items/search/?search=x", ws()), "key-admin", 200, Some("GET"))]
#[case::a_number_that_is_not_there("GET", format!("{}/work-items/{}-9999/", ws(), key_of_the_project()), "key-admin", 404, Some("GET"))]
// no view answered these: the URLconf has no route for them
#[case::an_unknown_route("GET", "/api/v1/this-route-does-not-exist/".to_string(), "key-admin", 404, None)]
#[case::an_id_that_is_no_uuid("GET", format!("{}/projects/not-a-uuid/", ws()), "key-admin", 404, None)]
fn an_answer_carries_the_methods_of_the_view_that_made_it(
    #[case] method: &str,
    #[case] uri: String,
    #[case] key: &str,
    #[case] status: u16,
    #[case] allow: Option<&str>,
) {
    let w = real_world();
    let body = json!({});
    let got = if method == "POST" && status == 400 {
        send_json(&w.router, key, method, &uri, &body)
    } else if key.is_empty() {
        send(&w.router, method, &uri, &[])
    } else {
        send_as(&w.router, key, method, &uri)
    };
    assert_eq!(got.status, status, "{method} {uri}: {}", got.body);
    assert_eq!(got.header("allow"), allow, "{method} {uri}");
}
