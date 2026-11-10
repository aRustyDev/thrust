//! The projects of Plane's v1 API (issue #58): list, lite list, create, retrieve, change, delete, archive and unarchive, through the
//! crate's own router over the memory adapters.
//!
//! What must hold, each pinned by the oracle's corpus (`projects.json`, `errors.json`) or read from the pin's source: the body's keys;
//! a caller sees the projects they are in and the public ones, whatever their role in the workspace; a creation makes the caller the
//! project's admin and is for a member or an admin of the workspace; a change, a deletion and archiving are for a project admin or a
//! member of the project who administers the workspace, and a project that is not there is a 403 for them; DRF's field messages; the
//! pin's two odd 409s; and that every answer past the permission carries the rate headers.
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

/// The keys of a project's body (`ProjectSerializer`): 43, and `sort_order` in a list.
const PROJECT_KEYS: [&str; 43] = [
    "archive_in",
    "archived_at",
    "close_in",
    "cover_image",
    "cover_image_asset",
    "cover_image_url",
    "created_at",
    "created_by",
    "cycle_view",
    "default_assignee",
    "default_state",
    "deleted_at",
    "description",
    "description_html",
    "description_text",
    "emoji",
    "estimate",
    "external_id",
    "external_source",
    "guest_view_all_features",
    "icon_prop",
    "id",
    "identifier",
    "intake_view",
    "is_deployed",
    "is_issue_type_enabled",
    "is_member",
    "is_time_tracking_enabled",
    "issue_views_view",
    "logo_props",
    "member_role",
    "module_view",
    "name",
    "network",
    "page_view",
    "project_lead",
    "timezone",
    "total_cycles",
    "total_members",
    "total_modules",
    "updated_at",
    "updated_by",
    "workspace",
];

const LITE_KEYS: [&str; 9] =
    ["archived_at", "cover_image", "cover_image_url", "description", "emoji", "icon_prop", "id", "identifier", "name"];

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

fn set(of: &[&str]) -> BTreeSet<String> {
    of.iter().map(|s| (*s).to_string()).collect()
}

fn projects() -> String {
    format!("/api/v1/workspaces/{SLUG}/projects/")
}

fn project(id: impl std::fmt::Display) -> String {
    format!("{}{id}/", projects())
}

fn cast() -> String {
    project(Uuid::from_u128(PROJECT).hyphenated())
}

fn get(w: &World, who: Who, uri: &str) -> Reply {
    send_as(&w.router, who.key(), "GET", uri)
}

fn call(w: &World, who: Who, method: &str, uri: &str, body: &Value) -> Reply {
    send_json(&w.router, who.key(), method, uri, body)
}

fn names(r: &Reply) -> Vec<String> {
    r.body["results"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

fn scope_of(w: &World, who: Who) -> WorkspaceScope {
    block_on(authorize_workspace(&w.app, &Principal { user_id: who.id() }, &WorkspaceSlug::new(SLUG))).unwrap()
}

/// A project made through core by `who` (v1's own creation cannot make a secret one), whose creator is its admin.
fn made(w: &World, who: Who, name: &str, key: &str, network: Option<ProjectNetwork>) -> String {
    let patch =
        ProjectPatch { name: Some(name.into()), identifier: Some(key.into()), network, ..ProjectPatch::default() };
    let d = block_on(create_project(&w.app, &scope_of(w, who), patch)).unwrap();
    project(d.listing.project.id.0.hyphenated())
}

// --- the body -----------------------------------------------------------------------------------------------------------------

#[test]
fn a_project_has_the_pins_keys_in_a_detail_a_list_and_a_lite_list() {
    let w = real_world();
    let detail = get(&w, Who::Admin, &cast());
    assert_eq!(detail.status, 200);
    assert_eq!(keys(&detail.body), set(&PROJECT_KEYS), "a detail has no `sort_order`");
    assert!(detail.has_rate_headers());

    let list = get(&w, Who::Admin, &projects());
    let entry = &list.body["results"][0];
    let mut with_order = set(&PROJECT_KEYS);
    with_order.insert("sort_order".into());
    assert_eq!(keys(entry), with_order, "a list entry has the caller's place besides");

    let lite = get(&w, Who::Admin, &format!("/api/v1/workspaces/{SLUG}/projects-lite/"));
    assert_eq!(keys(&lite.body["results"][0]), set(&LITE_KEYS));
}

#[test]
fn a_project_says_what_it_is_and_what_the_caller_is_in_it() {
    let w = real_world();
    let p = get(&w, Who::Member, &cast()).body;
    assert_eq!(p["id"], json!(Uuid::from_u128(PROJECT)));
    assert_eq!(p["workspace"], json!(Uuid::from_u128(WS)));
    assert_eq!((p["name"].as_str(), p["network"].clone()), (Some("Cast Project"), json!(2)));
    assert_eq!((p["is_member"].clone(), p["member_role"].clone()), (json!(true), json!(15)));
    assert_eq!(p["total_members"], json!(3), "the admin, the member and the guest");
    // Thrust has no cycles and no modules, and no deploy board
    assert_eq!(
        (p["total_cycles"].clone(), p["total_modules"].clone(), p["is_deployed"].clone()),
        (json!(0), json!(0), json!(false))
    );
    assert_eq!(p["deleted_at"], Value::Null);
    // a guest's role is 5, an admin's 20
    assert_eq!(get(&w, Who::Guest, &cast()).body["member_role"], json!(5));
    assert_eq!(get(&w, Who::Admin, &cast()).body["member_role"], json!(20));
}

// --- who sees which projects --------------------------------------------------------------------------------------------------

/// v1's rule, whatever the caller's role in the workspace: the projects they are in, and the public ones. The member makes a secret
/// project and the admin a public one: the admin of the workspace does not see the secret one, and the guest sees the public one.
#[test]
fn a_caller_sees_the_projects_they_are_in_and_the_public_ones() {
    let w = real_world();
    made(&w, Who::Member, "Secret S", "SEC", Some(ProjectNetwork::Secret));
    made(&w, Who::Admin, "Public P", "PUB", None);
    let seen = |who: Who| -> BTreeSet<String> { names(&get(&w, who, &projects())).into_iter().collect() };
    let strs = |of: &[&str]| of.iter().map(|s| (*s).to_string()).collect::<BTreeSet<_>>();
    assert_eq!(seen(Who::Admin), strs(&["Cast Project", "Public P"]));
    assert_eq!(seen(Who::Member), strs(&["Cast Project", "Public P", "Secret S"]));
    assert_eq!(seen(Who::Guest), strs(&["Cast Project", "Public P"]));
    // someone who is not in the workspace sees nothing, and is told so
    let outsider = get(&w, Who::Outsider, &projects());
    assert_eq!((outsider.status, outsider.body), (403, json!({ "detail": DENIED })));
}

/// The same rule for one project, as the pin's queryset has it: what the caller may not see is not found.
#[rstest]
#[case::a_member_of_a_secret_one(Who::Member, "secret", 200)]
#[case::an_admin_outside_a_secret_one(Who::Admin, "secret", 404)]
#[case::a_guest_outside_a_secret_one(Who::Guest, "secret", 404)]
#[case::a_guest_of_a_public_one_they_are_not_in(Who::Guest, "public", 200)]
#[case::an_admin_of_a_public_one_they_are_not_in(Who::Admin, "public", 200)]
fn a_project_is_found_by_whoever_is_in_it_or_if_it_is_public(
    #[case] who: Who,
    #[case] which: &str,
    #[case] status: u16,
) {
    let w = real_world();
    let secret = made(&w, Who::Member, "Secret S", "SEC", Some(ProjectNetwork::Secret));
    let public = made(&w, Who::Member, "Public P", "PUB", None);
    let r = get(&w, who, if which == "secret" { &secret } else { &public });
    assert_eq!(r.status, status);
    if status == 404 {
        assert_eq!(r.body, json!({ "error": NOT_FOUND }));
    } else {
        let in_it = who == Who::Member;
        assert_eq!((r.body["is_member"].clone(), r.body["member_role"].is_null()), (json!(in_it), !in_it));
    }
    assert!(r.has_rate_headers(), "an answer past the permission is counted");
}

#[test]
fn a_project_that_is_not_there_is_not_found_and_an_archived_one_is_found() {
    let w = real_world();
    let gone = get(&w, Who::Admin, &project(Uuid::from_u128(0xBAD).hyphenated()));
    assert_eq!((gone.status, gone.body), (404, json!({ "error": NOT_FOUND })));
    // an archived project is read as any other, with the time it was archived (the internal API hides it; v1's queryset does not)
    assert_eq!(call(&w, Who::Admin, "POST", &format!("{}archive/", cast()), &json!({})).status, 204);
    let r = get(&w, Who::Admin, &cast());
    assert_eq!(r.status, 200);
    assert!(r.body["archived_at"].is_string());
    let list = get(&w, Who::Admin, &projects());
    assert!(list.body["results"][0]["archived_at"].is_string(), "and the list has it too");
}

/// A DRF datetime: UTC, six digits of microseconds, `Z`. The world's clock stands at 1 760 000 000, which is this instant.
#[test]
fn a_project_made_now_has_the_times_the_pin_writes() {
    let w = real_world();
    let uri = made(&w, Who::Member, "Timed", "TIM", None);
    let p = get(&w, Who::Member, &uri).body;
    assert_eq!(p["created_at"], json!("2025-10-09T08:53:20.000000Z"));
    assert_eq!(p["updated_at"], json!("2025-10-09T08:53:20.000000Z"));
    // and one that is archived says when, in the same form
    assert_eq!(call(&w, Who::Member, "POST", &format!("{uri}archive/"), &json!({})).status, 204);
    assert_eq!(get(&w, Who::Member, &uri).body["archived_at"], json!("2025-10-09T08:53:20.000000Z"));
}

/// The list takes `updated_at` as an order, in both directions. The clock is fixed, so the cast project is given a time of its own,
/// later than the other's. The default order is the member rows' places, which are all the same (65535), so the projects come in the
/// order they were made: the cast project first. A list that ignored the key would answer that way for the ascending request, and the
/// ascending order expected here is the opposite, so ignoring the key shows.
#[test]
fn the_list_is_ordered_by_when_a_project_was_changed() {
    let w = real_world();
    made(&w, Who::Member, "Other", "OTH", None);
    let admin = scope_of(&w, Who::Admin);
    let mut row = block_on(w.app.ports.db.project_detail(&admin, ProjectId(Uuid::from_u128(PROJECT))))
        .unwrap()
        .unwrap()
        .listing
        .project;
    row.updated_at = thrust_contracts::chrono::DateTime::from_timestamp(NOW + 100, 0).unwrap();
    assert!(block_on(w.app.ports.db.update_project(&admin, &row)).unwrap());

    let order = |q: &str| names(&get(&w, Who::Member, &format!("{}?order_by={q}", projects())));
    assert_eq!(order("updated_at"), ["Other", "Cast Project"]);
    assert_eq!(order("-updated_at"), ["Cast Project", "Other"]);
    assert_eq!(
        order("sort_order"),
        ["Cast Project", "Other"],
        "the default: equal places, so the order they were made in"
    );
}

/// A new project has a random default icon and colour (the corpus's `create-scratch`: `logo_props` is `{in_use: "icon", icon:
/// {name, color}}` with the two values erased). The name and the colour are drawn from the pin's lists, and a `logo_props` that the
/// body gives is ignored, as the pin ignores it (measured with the emoji form and with null). Several creations show that the
/// choice is drawn and not fixed, and that it stays within the lists.
#[test]
fn a_new_project_gets_a_default_icon_from_the_pins_lists_and_ignores_the_one_the_body_gives() {
    use thrust_api_v1::{DEFAULT_COLORS, DEFAULT_ICONS};
    let w = real_world();
    let mut seen = std::collections::BTreeSet::new();
    for n in 0..24 {
        let given = if n % 2 == 0 { json!({"in_use": "emoji", "emoji": {"value": "128512"}}) } else { Value::Null };
        let body = json!({"name": format!("Icon {n}"), "identifier": format!("IC{n}"), "logo_props": given});
        let r = call(&w, Who::Member, "POST", &projects(), &body);
        assert_eq!(r.status, 201, "{}", r.body);
        let logo = &r.body["logo_props"];
        assert_eq!(logo["in_use"], json!("icon"), "{logo}");
        let (name, color) = (logo["icon"]["name"].as_str().unwrap(), logo["icon"]["color"].as_str().unwrap());
        assert!(DEFAULT_ICONS.contains(&name), "{name} is not one of the pin's icons");
        assert!(DEFAULT_COLORS.contains(&color), "{color} is not one of the pin's colours");
        assert_eq!(logo.as_object().unwrap().len(), 2, "only `in_use` and `icon`");
        seen.insert((name.to_string(), color.to_string()));
    }
    assert!(seen.len() > 1, "24 creations made the same icon: the choice is not drawn");
}

/// A project's `sort_order` in the list is the place of the CALLER'S MEMBER ROW (`project_members.sort_order`), which is 65535 for
/// every member that nothing has moved, however old or new the project: the corpus's `list` and `list-archived-visible` show 65535.0
/// for each project the owner is in, a project made in the middle of the run included, and list them in the order they were made.
/// (The compat plane's sidebar keeps another place, in the person's own settings, which puts a new project first: it is not this.)
#[test]
fn a_projects_sort_order_is_the_place_of_the_callers_member_row() {
    let w = real_world();
    made(&w, Who::Member, "Other", "OTH", None);
    let list = get(&w, Who::Member, &projects());
    let places: Vec<(String, Value)> = list.body["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["name"].as_str().unwrap().to_string(), p["sort_order"].clone()))
        .collect();
    assert_eq!(places, [("Cast Project".to_string(), json!(65535.0)), ("Other".to_string(), json!(65535.0))]);
}

// --- the list's order, projection and pages -----------------------------------------------------------------------------------

#[test]
fn the_list_is_ordered_by_what_the_pin_allows_and_by_the_default_for_anything_else() {
    let w = real_world();
    made(&w, Who::Member, "Secret S", "SEC", Some(ProjectNetwork::Secret));
    made(&w, Who::Member, "Public P", "PUB", None);
    let order = |q: &str| names(&get(&w, Who::Member, &format!("{}?order_by={q}", projects())));
    assert_eq!(order("name"), ["Cast Project", "Public P", "Secret S"]);
    assert_eq!(order("-name"), ["Secret S", "Public P", "Cast Project"]);
    // the secret one is network 0, the public ones 2: the order is the number's, not the word's
    assert_eq!(order("network").first().map(String::as_str), Some("Secret S"));
    assert_eq!(order("-network").last().map(String::as_str), Some("Secret S"));
    // the ones that are made later are newer, and the member's place puts the ones with none last
    assert_eq!(order("-created_at").last().map(String::as_str), Some("Cast Project"));
    // anything else is the default, `sort_order`: the same as asking for it, and as asking for nothing
    let default = names(&get(&w, Who::Member, &projects()));
    for other in ["sort_order", "nonsense", "--name", "-", "password", "id"] {
        assert_eq!(order(other), default, "{other}");
    }
}

#[test]
fn fields_keep_the_keys_named_and_ignore_the_rest() {
    let w = real_world();
    let r = get(&w, Who::Admin, &format!("{}?fields=id,name,identifier", projects()));
    assert_eq!(keys(&r.body["results"][0]), set(&["id", "name", "identifier"]));
    let r = get(&w, Who::Admin, &format!("{}?fields=id,nothing", projects()));
    assert_eq!(keys(&r.body["results"][0]), set(&["id"]));
    let r = get(&w, Who::Admin, &format!("{}?fields=name", cast()));
    assert_eq!(keys(&r.body), set(&["name"]));
    // a `fields` with nothing in it is all of them
    let r = get(&w, Who::Admin, &format!("{}?fields=", cast()));
    assert_eq!(keys(&r.body), set(&PROJECT_KEYS));
}

#[test]
fn following_the_cursors_of_a_list_visits_each_project_once() {
    let w = real_world();
    for i in 0..5 {
        made(&w, Who::Member, &format!("Extra {i}"), &format!("X{i}"), None);
    }
    let all = names(&get(&w, Who::Member, &format!("{}?order_by=name", projects())));
    assert_eq!(all.len(), 6);
    let (mut seen, mut cursor) = (Vec::new(), String::new());
    loop {
        let r = get(&w, Who::Member, &format!("{}?order_by=name&per_page=2{cursor}", projects()));
        assert_eq!(r.status, 200);
        assert_eq!((r.body["total_results"].clone(), r.body["total_pages"].clone()), (json!(6), json!(3)));
        seen.extend(names(&r));
        if r.body["next_page_results"] == json!(false) {
            break;
        }
        cursor = format!("&cursor={}", r.body["next_cursor"].as_str().unwrap());
    }
    assert_eq!(seen, all);
}

/// What the corpus records for a list that is asked wrongly (`pagination.json`): the 400s and the 500 are counted, so they carry the
/// rate headers.
#[rstest]
#[case("per_page=abc", 400, json!({"detail": "Invalid per_page parameter."}))]
#[case("per_page=1001", 400, json!({"detail": "Invalid per_page value. Cannot exceed 1000."}))]
#[case("cursor=garbage", 400, json!({"detail": "Invalid cursor parameter."}))]
#[case("per_page=0", 500, json!({"error": "Something went wrong please try again later"}))]
#[case("per_page=-1", 500, json!({"error": "Something went wrong please try again later"}))]
fn a_list_asked_wrongly_is_refused_as_the_pin_refuses_it(
    #[case] query: &str,
    #[case] status: u16,
    #[case] body: Value,
) {
    let w = real_world();
    for uri in [format!("{}?{query}", projects()), format!("/api/v1/workspaces/{SLUG}/projects-lite/?{query}")] {
        let r = get(&w, Who::Admin, &uri);
        assert_eq!((r.status, &r.body), (status, &body), "{uri}");
        assert!(r.has_rate_headers(), "{uri}");
    }
}

#[test]
fn the_lite_list_leaves_out_archived_projects_unless_asked() {
    let w = real_world();
    let other = made(&w, Who::Member, "Other", "OTH", None);
    assert_eq!(call(&w, Who::Member, "POST", &format!("{other}archive/"), &json!({})).status, 204);
    let lite = |q: &str| names(&get(&w, Who::Member, &format!("/api/v1/workspaces/{SLUG}/projects-lite/{q}")));
    assert_eq!(lite(""), ["Cast Project"]);
    for yes in ["true", "True", "TRUE", "1"] {
        assert_eq!(lite(&format!("?include_archived={yes}")).len(), 2, "{yes}");
    }
    for no in ["false", "0", "yes", "", "nonsense"] {
        assert_eq!(lite(&format!("?include_archived={no}")).len(), 1, "{no}");
    }
    // newest first by default
    assert_eq!(lite("?include_archived=true"), ["Other", "Cast Project"]);
    // the full list shows both, whatever
    assert_eq!(names(&get(&w, Who::Member, &projects())).len(), 2);
}

// --- making a project ---------------------------------------------------------------------------------------------------------

#[test]
fn a_member_makes_a_project_and_becomes_its_admin() {
    let w = real_world();
    let r = call(
        &w,
        Who::Member,
        "POST",
        &projects(),
        &json!({"name": "  New One  ", "identifier": "nw1", "description": "d"}),
    );
    assert_eq!(r.status, 201);
    assert!(r.has_rate_headers());
    assert_eq!(keys(&r.body), set(&PROJECT_KEYS), "a creation's answer has no `sort_order`");
    // a name is trimmed, a key is saved in upper case, and the project is public and the caller is its admin
    assert_eq!((r.body["name"].as_str(), r.body["identifier"].as_str()), (Some("New One"), Some("NW1")));
    assert_eq!(
        (r.body["network"].clone(), r.body["member_role"].clone(), r.body["is_member"].clone()),
        (json!(2), json!(20), json!(true))
    );
    assert_eq!((r.body["total_members"].clone(), r.body["created_by"].clone()), (json!(1), json!(Who::Member.id().0)));
    assert_eq!(r.body["description"], json!("d"));

    // and as its admin they change it, where the workspace's admin, who is not in it, may not
    let id = r.body["id"].as_str().unwrap().to_string();
    let uri = project(&id);
    assert_eq!(call(&w, Who::Member, "PATCH", &uri, &json!({"description": "mine"})).status, 200);
    let refused = call(&w, Who::Admin, "PATCH", &uri, &json!({"description": "theirs"}));
    assert_eq!((refused.status, refused.body), (403, json!({ "detail": DENIED })));
}

#[rstest]
#[case::a_guest(Who::Guest, 403)]
#[case::a_member(Who::Member, 201)]
#[case::an_admin(Who::Admin, 201)]
#[case::a_stranger(Who::Outsider, 403)]
fn a_member_or_an_admin_of_the_workspace_makes_a_project(#[case] who: Who, #[case] status: u16) {
    let w = real_world();
    let r = call(&w, who, "POST", &projects(), &json!({"name": "Fresh", "identifier": "FRS"}));
    assert_eq!(r.status, status);
    if status == 403 {
        assert_eq!(r.body, json!({ "detail": DENIED }));
        assert!(!r.has_rate_headers(), "a refused request is not counted");
    }
}

/// DRF's field messages for a creation. The first two come from the corpus (`create-missing-name`, `create-identifier-too-long`).
#[rstest]
#[case::no_name(json!({"identifier": "NON"}), json!({"name": ["This field is required."]}))]
#[case::identifier_too_long(json!({"identifier": "ABCDEFGHIJKLMNOP", "name": "Long Ident"}), json!({"identifier": ["Ensure this field has no more than 12 characters."]}))]
#[case::nothing(json!({}), json!({"name": ["This field is required."], "identifier": ["This field is required."]}))]
#[case::blank_name(json!({"name": "  ", "identifier": "BLK"}), json!({"name": ["This field may not be blank."]}))]
#[case::null_name(json!({"name": null, "identifier": "NUL"}), json!({"name": ["This field may not be null."]}))]
#[case::a_bool_for_a_name(json!({"name": true, "identifier": "BOO"}), json!({"name": ["Not a valid string."]}))]
#[case::a_list_for_a_name(json!({"name": ["a"], "identifier": "LST"}), json!({"name": ["Not a valid string."]}))]
#[case::too_many_months(json!({"name": "M", "identifier": "MON", "archive_in": 13}), json!({"archive_in": ["Ensure this value is less than or equal to 12."]}))]
#[case::too_few_months(json!({"name": "M", "identifier": "MON", "close_in": -1}), json!({"close_in": ["Ensure this value is greater than or equal to 0."]}))]
#[case::months_that_are_not_numbers(json!({"name": "M", "identifier": "MON", "archive_in": "x"}), json!({"archive_in": ["A valid integer is required."]}))]
#[case::not_a_boolean(json!({"name": "M", "identifier": "MON", "module_view": "maybe"}), json!({"module_view": ["Must be a valid boolean."]}))]
#[case::not_a_zone(json!({"name": "M", "identifier": "MON", "timezone": "Nope/Zone"}), json!({"timezone": ["\"Nope/Zone\" is not a valid choice."]}))]
#[case::not_a_uuid(json!({"name": "M", "identifier": "MON", "project_lead": "nope"}), json!({"project_lead": ["Must be a valid UUID."]}))]
#[case::a_lead_who_is_nobody(json!({"name": "M", "identifier": "MON", "project_lead": "00000000-0000-0000-0000-000000000bad"}), json!({"project_lead": ["Invalid pk \"00000000-0000-0000-0000-000000000bad\" - object does not exist."]}))]
#[case::several_at_once(json!({"identifier": "ABCDEFGHIJKLMNOP", "archive_in": 99}), json!({"name": ["This field is required."], "identifier": ["Ensure this field has no more than 12 characters."], "archive_in": ["Ensure this value is less than or equal to 12."]}))]
fn a_creation_with_a_field_wrong_is_refused_with_drfs_message(#[case] body: Value, #[case] errors: Value) {
    let w = real_world();
    let r = call(&w, Who::Member, "POST", &projects(), &body);
    assert_eq!((r.status, &r.body), (400, &errors));
    assert!(r.has_rate_headers());
    // and nothing was made
    assert_eq!(names(&get(&w, Who::Member, &projects())), ["Cast Project"]);
}

/// What the serializer's `validate()` refuses, one thing at a time, in its order: the special characters of a name, then of a key,
/// then a lead who is not in the workspace, then a default assignee who is not. It runs only when every field was fine.
#[rstest]
#[case::a_name(json!({"name": "Bad.Name", "identifier": "BAD"}), 400, json!({"non_field_errors": ["Project name cannot contain special characters."]}))]
#[case::a_key(json!({"name": "Fine", "identifier": "A-B"}), 400, json!({"non_field_errors": ["Project identifier cannot contain special characters."]}))]
#[case::the_name_first(json!({"name": "Bad.Name", "identifier": "A-B"}), 400, json!({"non_field_errors": ["Project name cannot contain special characters."]}))]
#[case::a_lead_outside_the_workspace(json!({"name": "Fine", "identifier": "FIN", "project_lead": "00000000-0000-0000-0000-000000000013"}), 400, json!({"project_lead": "The provided user is not a member of this workspace."}))]
#[case::an_assignee_outside_the_workspace(json!({"name": "Fine", "identifier": "FIN", "default_assignee": "00000000-0000-0000-0000-000000000013"}), 400, json!({"non_field_errors": ["Default assignee should be a user in the workspace"]}))]
#[case::a_field_error_hides_validate(json!({"name": "Bad.Name", "identifier": "THIS-IS-WAY-TOO-LONG"}), 400, json!({"identifier": ["Ensure this field has no more than 12 characters."]}))]
fn the_serializers_validate_refuses_one_thing_at_a_time(
    #[case] body: Value,
    #[case] status: u16,
    #[case] errors: Value,
) {
    let w = real_world();
    let r = call(&w, Who::Member, "POST", &projects(), &body);
    assert_eq!((r.status, &r.body), (status, &errors));
}

/// Where Plane is lenient, v1 is: a number is a name (`create-wrong-type`, 201), a field it does not know is ignored (a `network`, an
/// `id`, a `created_by`), a lead who is a member is made an admin as well, and a null for a field that allows one clears it.
#[test]
fn a_creation_is_as_lenient_as_the_pin() {
    let w = real_world();
    let r = call(&w, Who::Member, "POST", &projects(), &json!({"name": 123, "identifier": "WTY"}));
    assert_eq!((r.status, r.body["name"].clone()), (201, json!("123")));

    let r = call(
        &w,
        Who::Member,
        "POST",
        &projects(),
        &json!({
            "name": "Lenient", "identifier": "LEN", "network": 0, "id": "00000000-0000-0000-0000-000000000bad",
            "created_by": "00000000-0000-0000-0000-000000000013", "deleted_at": "2020-01-01T00:00:00Z",
            "project_lead": Who::Admin.id().0, "emoji": null, "external_id": "e-1", "external_source": "gh",
            "archive_in": "6.0", "timezone": "Europe/Paris", "module_view": "yes",
        }),
    );
    assert_eq!(r.status, 201, "{}", r.body);
    assert_eq!(r.body["network"], json!(2), "v1 has no `network` to set");
    assert_ne!(r.body["id"], json!("00000000-0000-0000-0000-000000000bad"));
    assert_eq!(r.body["created_by"], json!(Who::Member.id().0));
    assert_eq!(r.body["deleted_at"], Value::Null);
    assert_eq!(r.body["project_lead"], json!(Who::Admin.id().0));
    assert_eq!(
        (
            r.body["archive_in"].clone(),
            r.body["timezone"].clone(),
            r.body["module_view"].clone(),
            r.body["external_id"].clone()
        ),
        (json!(6), json!("Europe/Paris"), json!(true), json!("e-1"))
    );
    // the lead is a member of the project as well, and its admin: the creator and the lead
    assert_eq!(r.body["total_members"], json!(2));
}

/// The pin answers any integrity error that says "already exists" with the name's message, and so a taken KEY is answered as a
/// taken name (the corpus's `create-duplicate-identifier` is a 409 `{"name": ...}`).
#[test]
fn a_taken_name_or_key_is_a_409_with_the_names_message() {
    let w = real_world();
    let first = call(&w, Who::Member, "POST", &projects(), &json!({"name": "Oracle Scratch", "identifier": "SCR"}));
    assert_eq!(first.status, 201);
    let taken = json!({"name": "The project name is already taken"});
    for body in [
        json!({"name": "Oracle Scratch", "identifier": "SCT"}),
        json!({"name": "Oracle Scratch Two", "identifier": "SCR"}),
        json!({"name": "Oracle Scratch Two", "identifier": "scr"}),
        json!({"name": "Oracle Scratch", "identifier": "SCR"}),
    ] {
        let r = call(&w, Who::Member, "POST", &projects(), &body);
        assert_eq!((r.status, &r.body), (409, &taken), "{body}");
        assert!(r.has_rate_headers());
    }
    // exact match: another case is another name
    assert_eq!(
        call(&w, Who::Member, "POST", &projects(), &json!({"name": "oracle scratch", "identifier": "SCU"})).status,
        201
    );
}

/// A body that is not an object, as the corpus records (`errors.json`): the wrong content type, JSON that does not parse, a value that
/// is not an object. A list is read as the pin's `{**request.data}` fails on it: a 500 that this view's catch-all words.
#[test]
fn a_body_that_is_not_an_object_is_refused_as_the_pin_refuses_it() {
    let w = real_world();
    let post = |ct: &str, body: &str| {
        send_raw(
            &w.router,
            "POST",
            &projects(),
            &[("x-api-key", Who::Member.key()), ("content-type", ct)],
            body.as_bytes().to_vec(),
        )
    };
    let r = post("text/plain", "name=x");
    assert_eq!((r.status, r.body), (415, json!({"detail": "Unsupported media type \"text/plain\" in request."})));

    let r = post("application/json", "{bad json");
    assert_eq!(r.status, 400);
    assert!(r.body["detail"].as_str().unwrap().starts_with("JSON parse error - "), "{}", r.body);

    let r = post("application/json", "\"just a string\"");
    assert_eq!((r.status, r.body), (400, json!({"detail": "Request body must be a JSON object or array."})));

    let r = post("application/json", "[1, 2]");
    assert_eq!((r.status, r.body), (500, json!({"error": "An unexpected error occurred"})));

    // a charset after the type is still JSON, and an empty body is no data: the two fields are then missing
    let r = post("application/json; charset=utf-8", "{\"name\": \"Charset\", \"identifier\": \"CHR\"}");
    assert_eq!(r.status, 201);
    let r = send_raw(&w.router, "POST", &projects(), &[("x-api-key", Who::Member.key())], Vec::new());
    assert_eq!(r.status, 400);
    assert_eq!(r.body, json!({"name": ["This field is required."], "identifier": ["This field is required."]}));
    assert!(r.has_rate_headers());
}

// --- changing a project -------------------------------------------------------------------------------------------------------

#[test]
fn a_change_sets_what_it_names_and_leaves_the_rest() {
    let w = real_world();
    let before = get(&w, Who::Admin, &cast()).body;
    let r = call(&w, Who::Admin, "PATCH", &cast(), &json!({"description": "patched", "emoji": "x", "archive_in": 3}));
    assert_eq!(r.status, 200);
    assert!(r.has_rate_headers());
    assert_eq!(keys(&r.body), set(&PROJECT_KEYS));
    let mut want = before.as_object().unwrap().clone();
    for (k, v) in [("description", json!("patched")), ("emoji", json!("x")), ("archive_in", json!(3))] {
        want.insert(k.into(), v);
    }
    // who changed it and when are the only other differences
    let mut got = r.body.as_object().unwrap().clone();
    assert_eq!(got.remove("updated_by"), Some(json!(Who::Admin.id().0)));
    want.remove("updated_by");
    got.remove("updated_at");
    want.remove("updated_at");
    assert_eq!(got, want);
}

/// `network` is no field of v1's serializers, so a change that names one (`patch-scratch-invalid-network`: 200) changes nothing.
#[test]
fn a_change_ignores_a_network() {
    let w = real_world();
    for network in [json!(0), json!(99), json!("secret")] {
        let r = call(&w, Who::Admin, "PATCH", &cast(), &json!({ "network": network }));
        assert_eq!((r.status, r.body["network"].clone()), (200, json!(2)), "{network}");
    }
}

#[test]
fn a_change_cannot_take_the_name_or_key_of_another_project() {
    let w = real_world();
    made(&w, Who::Member, "Other", "OTH", None);
    let taken = json!({"name": "The project name is already taken"});
    for body in [json!({"name": "Other"}), json!({"identifier": "oth"}), json!({"identifier": "OTH"})] {
        let r = call(&w, Who::Admin, "PATCH", &cast(), &body);
        assert_eq!((r.status, &r.body), (409, &taken), "{body}");
    }
    // its own name and key are not another's
    assert_eq!(call(&w, Who::Admin, "PATCH", &cast(), &json!({"name": "Cast Project"})).status, 200);
}

#[test]
fn an_archived_project_is_not_changed_and_that_is_told_before_the_fields_are_read() {
    let w = real_world();
    assert_eq!(call(&w, Who::Admin, "POST", &format!("{}archive/", cast()), &json!({})).status, 204);
    let archived = json!({"error": "Archived project cannot be updated"});
    for body in [json!({"description": "x"}), json!({"name": "Bad.Name"}), json!({"archive_in": 99}), json!({})] {
        let r = call(&w, Who::Admin, "PATCH", &cast(), &body);
        assert_eq!((r.status, &r.body), (400, &archived), "{body}");
        assert!(r.has_rate_headers());
    }
    // but a body that cannot be read at all is told first, as the pin reads it before it looks
    let r = send_raw(
        &w.router,
        "PATCH",
        &cast(),
        &[("x-api-key", Who::Admin.key()), ("content-type", "text/plain")],
        b"x".to_vec(),
    );
    assert_eq!(r.status, 415);
    // and unarchived it is changed again
    assert_eq!(call(&w, Who::Admin, "DELETE", &format!("{}archive/", cast()), &json!({})).status, 204);
    assert_eq!(call(&w, Who::Admin, "PATCH", &cast(), &json!({"description": "again"})).status, 200);
}

/// A default state is one of the project's own (the triage one is not); an estimate names a row that no project here has.
#[test]
fn a_change_names_a_default_state_of_the_project_and_an_estimate_of_none() {
    let w = real_world();
    let todo = Uuid::from_u128(STATE_TODO);
    let r = call(&w, Who::Admin, "PATCH", &cast(), &json!({ "default_state": todo }));
    assert_eq!((r.status, r.body["default_state"].clone()), (200, json!(todo)));

    // a state that is nowhere, another project's, and the intake state of this one are no state of it
    let other = made(&w, Who::Member, "Other", "OTH", None);
    let others_state = block_on(w.app.ports.db.workspace_states(&scope_of(&w, Who::Member)))
        .unwrap()
        .into_iter()
        .find(|s| s.name == "Todo" && s.project_id.0.to_string() != Uuid::from_u128(PROJECT).to_string())
        .unwrap()
        .id;
    for not_ours in [Uuid::from_u128(0xDEAD), others_state, Uuid::from_u128(STATE_TRIAGE)] {
        let r = call(&w, Who::Admin, "PATCH", &cast(), &json!({ "default_state": not_ours }));
        assert_eq!(r.status, 400, "{not_ours}");
        assert_eq!(
            r.body,
            json!({"default_state": [format!("Invalid pk \"{not_ours}\" - object does not exist.")]}),
            "{not_ours}"
        );
    }
    // a refusal changed nothing, and null clears it
    assert_eq!(get(&w, Who::Admin, &cast()).body["default_state"], json!(todo));
    assert_eq!(
        call(&w, Who::Admin, "PATCH", &cast(), &json!({"default_state": null})).body["default_state"],
        Value::Null
    );
    let _ = other;

    let id = Uuid::from_u128(0xE57);
    let r = call(&w, Who::Admin, "PATCH", &cast(), &json!({ "estimate": id }));
    assert_eq!(
        (r.status, r.body),
        (400, json!({"estimate": [format!("Invalid pk \"{id}\" - object does not exist.")]}))
    );
    assert_eq!(call(&w, Who::Admin, "PATCH", &cast(), &json!({"estimate": null})).status, 200);
}

/// The pin's permission for a write: a project admin, or a member of the project who administers the workspace. Everyone else, a
/// project that is not there included, is refused (403) and not counted. The matrix is every caller against the three writes, on a
/// project the member made (so the workspace's admin is not in it) and on the cast project (where the admin is its admin).
#[test]
fn a_write_is_for_a_project_admin_or_a_project_member_who_administers_the_workspace() {
    let writes = |w: &World, who: Who, uri: &str| -> Vec<u16> {
        // each write on a project of its own, so that one deleted does not take the next with it
        let mut statuses = Vec::new();
        let patch = call(w, who, "PATCH", uri, &json!({"description": "x"})).status;
        statuses.push(patch);
        statuses.push(call(w, who, "POST", &format!("{uri}archive/"), &json!({})).status);
        statuses.push(call(w, who, "DELETE", &format!("{uri}archive/"), &json!({})).status);
        statuses
    };
    // (the caller, whether they are the project's admin) on the project the member made, and on the cast project
    let on_members_project =
        [(Who::Member, true), (Who::Admin, false), (Who::Guest, false), (Who::Outsider, false), (Who::Inactive, false)];
    for (who, allowed) in on_members_project {
        let w = real_world();
        let uri = made(&w, Who::Member, "Members", "MEM", None);
        let got = writes(&w, who, &uri);
        let want: Vec<u16> = if allowed { vec![200, 204, 204] } else { vec![403, 403, 403] };
        assert_eq!(got, want, "{who:?} on the member's project");
    }
    // on the cast project the admin is the project's admin, the member and the guest are not
    for (who, allowed) in [(Who::Admin, true), (Who::Member, false), (Who::Guest, false)] {
        let w = real_world();
        let got = writes(&w, who, &cast());
        let want: Vec<u16> = if allowed { vec![200, 204, 204] } else { vec![403, 403, 403] };
        assert_eq!(got, want, "{who:?} on the cast project");
    }
    // a project that is not there is refused to the workspace's admin as to everyone (the corpus's `patch-unknown`)
    let w = real_world();
    let nowhere = project(Uuid::from_u128(0xBAD).hyphenated());
    for who in [Who::Admin, Who::Member, Who::Guest] {
        let r = call(&w, who, "PATCH", &nowhere, &json!({"description": "x"}));
        assert_eq!((r.status, r.body), (403, json!({ "detail": DENIED })), "{who:?}");
        assert_eq!(call(&w, who, "DELETE", &nowhere, &json!({})).status, 403, "{who:?}");
    }
    // a refused write is not counted
    let r = call(&w, Who::Guest, "PATCH", &cast(), &json!({"description": "x"}));
    assert!(!r.has_rate_headers());
}

// --- deleting and archiving ---------------------------------------------------------------------------------------------------

#[test]
fn a_deleted_project_is_gone_and_its_name_and_key_are_free() {
    let w = real_world();
    let uri = made(&w, Who::Member, "Doomed", "DMD", None);
    assert_eq!(get(&w, Who::Member, &uri).status, 200);

    let r = call(&w, Who::Member, "DELETE", &uri, &json!({}));
    assert_eq!(r.status, 204);
    assert_eq!(r.body, Value::Null, "a 204 has no body");
    assert!(r.has_rate_headers());

    // not found to a read (`detail-after-delete`), and refused to a second deletion (`delete-again`: no membership is left to find)
    let r = get(&w, Who::Member, &uri);
    assert_eq!((r.status, r.body), (404, json!({ "error": NOT_FOUND })));
    let again = call(&w, Who::Member, "DELETE", &uri, &json!({}));
    assert_eq!((again.status, again.body), (403, json!({ "detail": DENIED })));
    assert!(!names(&get(&w, Who::Member, &projects())).contains(&"Doomed".to_string()));
    assert!(!names(&get(&w, Who::Member, &format!("/api/v1/workspaces/{SLUG}/projects-lite/?include_archived=true")))
        .contains(&"Doomed".to_string()));

    // the name and the key are free again
    let r = call(&w, Who::Member, "POST", &projects(), &json!({"name": "Doomed", "identifier": "DMD"}));
    assert_eq!(r.status, 201);
}

#[test]
fn archiving_and_unarchiving_move_a_project_out_of_and_into_the_lite_list() {
    let w = real_world();
    let lite = format!("/api/v1/workspaces/{SLUG}/projects-lite/");
    assert_eq!(names(&get(&w, Who::Admin, &lite)), ["Cast Project"]);
    let r = call(&w, Who::Admin, "POST", &format!("{}archive/", cast()), &json!({}));
    assert_eq!((r.status, r.body), (204, Value::Null));
    assert!(names(&get(&w, Who::Admin, &lite)).is_empty());
    assert!(get(&w, Who::Admin, &cast()).body["archived_at"].is_string());
    // archiving it again is archiving it again
    assert_eq!(call(&w, Who::Admin, "POST", &format!("{}archive/", cast()), &json!({})).status, 204);
    let r = call(&w, Who::Admin, "DELETE", &format!("{}archive/", cast()), &json!({}));
    assert_eq!(r.status, 204);
    assert_eq!(names(&get(&w, Who::Admin, &lite)), ["Cast Project"]);
    assert_eq!(get(&w, Who::Admin, &cast()).body["archived_at"], Value::Null);
}
