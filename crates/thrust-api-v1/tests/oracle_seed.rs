//! The oracle's seed (`thrust-oracle-fixture`) loaded through `thrust_core::seed` into the memory adapter, and read through the
//! real `/api/v1` routes (issue #58). Whether the fixture has the right rows is `thrust-oracle-fixture`'s tests; whether a
//! database turns it into those rows, and the routes show them, is what is checked here.
//!
//! The expected answers are the oracle's own: the recorded exchanges of `tools/oracle-v1/corpus/` are read at run time, so no
//! status or body is copied into this file (a static comparison with recorded output is AGENTS.md testing rule 9 (c)).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use axum::Extension;
use common::*;
use serde_json::{json, Value};
use thrust_api_v1::V1Config;
use thrust_contracts::chrono::DateTime;
use thrust_core::seed;
use thrust_oracle_fixture::oracle_seed;

/// The router over an empty database, with the oracle's seed loaded at the instant the world's clock stands at.
fn loaded() -> (axum::Router, Value) {
    let app = empty_app();
    let s = oracle_seed(DateTime::from_timestamp(NOW, 0).expect("a valid instant"));
    futures_executor::block_on(seed(&app, &s.fixture)).expect("the oracle's fixture loads");
    let router = thrust_api_v1::router()
        .layer(Extension(app))
        .layer(Extension(V1Config { rate_limit: 100_000, rate_window_secs: 60 }));
    (router, s.manifest)
}

fn recorded(file: &str, exchange: &str) -> Value {
    let path = format!("{}/../../tools/oracle-v1/corpus/{file}", env!("CARGO_MANIFEST_DIR"));
    let corpus: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("the corpus file")).expect("JSON");
    corpus["exchanges"]
        .as_array()
        .and_then(|all| all.iter().find(|e| e["name"] == exchange))
        .unwrap_or_else(|| panic!("{file} has no exchange {exchange}"))
        .clone()
}

/// A recorded path with its placeholders (`<project:alpha>`) replaced by the ids of the manifest.
fn real_path(manifest: &Value, recorded_path: &str) -> String {
    let mut out = recorded_path.to_string();
    for (key, v) in manifest["projects"].as_object().expect("projects") {
        out = out.replace(&format!("<project:{key}>"), v["id"].as_str().expect("an id"));
    }
    out
}

fn token<'a>(manifest: &'a Value, who: &str) -> &'a str {
    manifest["tokens"][who].as_str().unwrap_or_else(|| panic!("the manifest has no token for {who}"))
}

/// Every recorded refusal that depends on nothing but the seed: who is asking, which workspace and project, and the role the
/// oracle gave them. The answer is the recorded one, status and body.
#[test]
fn the_recorded_refusals_are_what_the_loaded_seed_answers() {
    let (router, manifest) = loaded();
    let names = [
        "outsider-lists-workspace-projects",
        "outsider-reads-public-project",
        "member-reads-secret-project",
        "guest-creates-work-item",
        "unknown-workspace-slug",
    ];
    for name in names {
        let ex = recorded("errors.json", name);
        let req = &ex["request"];
        let path = real_path(&manifest, req["path"].as_str().expect("a path"));
        let key = token(&manifest, req["auth"].as_str().expect("an auth"));
        let method = req["method"].as_str().expect("a method");
        let got = if req["body"].is_null() {
            send_as(&router, key, method, &path)
        } else {
            send_json(&router, key, method, &path, &req["body"])
        };
        assert_eq!(i64::from(got.status), ex["response"]["status"].as_i64().expect("a status"), "{name}: status");
        assert_eq!(got.body, ex["response"]["body"], "{name}: body");
    }
}

/// The two tokens that Plane refuses (`expired`, `inactive`) are answered with the recorded 403, and the other five are let in
/// as the people they belong to. Matrix over the table of tokens, so a token added to it is covered.
#[test]
fn the_seeded_tokens_are_refused_or_let_in_as_the_oracle_recorded() {
    let (router, manifest) = loaded();
    let refusal = recorded("errors.json", "expired-api-key");
    assert_eq!(
        recorded("errors.json", "inactive-api-key")["response"],
        refusal["response"],
        "the oracle answers both alike"
    );
    let uri = "/api/v1/workspaces/oracle-ws/projects/";
    let mut refused = 0;
    for (who, _, _, expired, inactive) in thrust_oracle_fixture::TOKENS {
        let got = send_as(&router, token(&manifest, who), "GET", uri);
        if expired || inactive {
            refused += 1;
            assert_eq!(
                i64::from(got.status),
                refusal["response"]["status"].as_i64().expect("a status"),
                "{who}: status"
            );
            assert_eq!(got.body, refusal["response"]["body"], "{who}: body");
        } else if who == "outsider" {
            // a real user in no workspace: the recorded answer is the permission's
            let rec = recorded("errors.json", "outsider-lists-workspace-projects");
            assert_eq!(i64::from(got.status), rec["response"]["status"].as_i64().expect("a status"));
        } else {
            assert_eq!(got.status, 200, "{who} is let in");
        }
    }
    assert_eq!(refused, 2, "a pass that refused no token is not a pass");
}

/// The owner is in both projects and the member only in Alpha: Beta is secret.
#[test]
fn the_projects_each_caller_sees_are_the_ones_they_belong_to() {
    let (router, manifest) = loaded();
    let uri = "/api/v1/workspaces/oracle-ws/projects/";
    let identifiers = |who: &str| -> Vec<String> {
        let got = send_as(&router, token(&manifest, who), "GET", uri);
        assert_eq!(got.status, 200, "{who}");
        let mut ids: Vec<String> = got.body["results"]
            .as_array()
            .expect("results")
            .iter()
            .map(|p| p["identifier"].as_str().expect("an identifier").into())
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(identifiers("owner"), ["ORA", "ORB"]);
    assert_eq!(identifiers("member"), ["ORA"]);
    assert_eq!(identifiers("guest"), ["ORA"]);
}

/// What the seed numbered, the routes find by number: item `ORA-3` is the manifest's `i3`, and `ORA-6` is its sub-item.
#[test]
fn a_seeded_item_is_found_by_its_number_and_has_the_parent_the_seed_gave_it() {
    let (router, manifest) = loaded();
    let owner = token(&manifest, "owner");
    for (identifier, key) in [("ORA-1", "i1"), ("ORA-3", "i3"), ("ORA-9", "i9"), ("ORB-2", "b2")] {
        let got = send_as(&router, owner, "GET", &format!("/api/v1/workspaces/oracle-ws/work-items/{identifier}/"));
        assert_eq!(got.status, 200, "{identifier}");
        assert_eq!(got.body["id"], manifest["issues"][key]["id"], "{identifier} is {key}");
        assert_eq!(got.body["sequence_id"], manifest["issues"][key]["sequence_id"], "{identifier}");
    }
    let sub = send_as(&router, owner, "GET", "/api/v1/workspaces/oracle-ws/work-items/ORA-6/");
    assert_eq!(sub.body["parent"], manifest["issues"]["i3"]["id"]);
}

/// The id that a corpus placeholder (`<user:owner>`, `<issue:i3>`, `<label:alpha:docs>`, `<state:alpha:Todo>`) stands for.
fn resolve(manifest: &Value, placeholder: &str) -> Value {
    let inner = placeholder.strip_prefix('<').and_then(|p| p.strip_suffix('>')).expect("a placeholder");
    let (kind, key) = inner.split_once(':').expect("a kind and a key");
    match kind {
        "user" | "issue" | "project" => manifest[format!("{kind}s")][key]["id"].clone(),
        "label" | "state" => manifest[format!("{kind}s")][key].clone(),
        other => panic!("no placeholder of kind {other}"),
    }
}

/// Every row of Alpha's recorded list, as the loaded seed serves it: what the adapter kept of each item's state, priority, place,
/// people, labels, parent and dates. The corpus has times and dates normalised, so those are compared as set or not; the people
/// and the labels as sets, because their order in a list is the corpus comparison's business (a later stage of #58).
#[test]
fn the_seeded_work_items_are_listed_as_the_oracle_recorded() {
    let (router, manifest) = loaded();
    let ex = recorded("work_items.json", "list");
    let path = real_path(&manifest, ex["request"]["path"].as_str().expect("a path"));
    let got = send_as(&router, token(&manifest, ex["request"]["auth"].as_str().expect("an auth")), "GET", &path);
    assert_eq!(got.status, 200);
    let ours = got.body["results"].as_array().expect("results");
    let theirs = ex["response"]["body"]["results"].as_array().expect("recorded results");
    assert_eq!(theirs.len(), 9, "the recorded list is the nine of Alpha");
    assert_eq!(ours.len(), theirs.len());
    let set = |v: &Value| -> std::collections::BTreeSet<String> {
        v.as_array().expect("a list").iter().map(|x| x.to_string()).collect()
    };
    for recorded_item in theirs {
        let seq = recorded_item["sequence_id"].as_i64().expect("a number");
        let item =
            ours.iter().find(|o| o["sequence_id"].as_i64() == Some(seq)).unwrap_or_else(|| panic!("no item {seq}"));
        let which = format!("item {seq}");
        for field in ["name", "priority", "sort_order", "description_html", "created_by", "external_id"] {
            assert_eq!(item[field], recorded_item[field], "{which}: {field}");
        }
        assert_eq!(
            item["state"],
            resolve(&manifest, recorded_item["state"].as_str().expect("a state")),
            "{which}: state"
        );
        let parent = recorded_item["parent"].as_str().map_or(Value::Null, |p| resolve(&manifest, p));
        assert_eq!(item["parent"], parent, "{which}: parent");
        for field in ["completed_at", "start_date", "target_date"] {
            assert_eq!(item[field].is_null(), recorded_item[field].is_null(), "{which}: {field} is set or not");
        }
        let resolved = |v: &Value| -> Value {
            Value::Array(
                v.as_array()
                    .expect("a list")
                    .iter()
                    .map(|x| resolve(&manifest, x.as_str().expect("a placeholder")))
                    .collect(),
            )
        };
        assert_eq!(set(&item["assignees"]), set(&resolved(&recorded_item["assignees"])), "{which}: assignees");
        assert_eq!(set(&item["labels"]), set(&resolved(&recorded_item["labels"])), "{which}: labels");
    }
}

/// The people a work item names, expanded: who each is and what they are called, as the recorded list with `expand` shows them.
#[test]
fn the_people_of_the_seeded_work_items_are_expanded_as_the_oracle_recorded() {
    let (router, manifest) = loaded();
    let ex = recorded("work_items.json", "list-expand-assignees-labels-state");
    let query = ex["request"]["query"].as_object().expect("the recorded query");
    let qs: Vec<String> = query.iter().map(|(k, v)| format!("{k}={}", v.as_str().expect("text"))).collect();
    let path = format!("{}?{}", real_path(&manifest, ex["request"]["path"].as_str().expect("a path")), qs.join("&"));
    let got = send_as(&router, token(&manifest, ex["request"]["auth"].as_str().expect("an auth")), "GET", &path);
    assert_eq!(got.status, 200);
    // Each person as a `Value`, compared as a value: two objects with the same keys are equal however the keys are ordered, which
    // the text of them is not (a workspace build that turns on serde_json's `preserve_order` keeps the router's order, and the
    // corpus file's is alphabetical). The people of an item are in id order, because their order is the corpus comparison's business.
    let people = |results: &Value, resolve_ids: bool| -> std::collections::BTreeMap<i64, Vec<Value>> {
        results
            .as_array()
            .expect("results")
            .iter()
            .map(|item| {
                let mut each: Vec<Value> = item["assignees"]
                    .as_array()
                    .expect("assignees")
                    .iter()
                    .map(|p| {
                        let mut p = p.clone();
                        if resolve_ids {
                            p["id"] = resolve(&manifest, p["id"].as_str().expect("a placeholder"));
                        }
                        p
                    })
                    .collect();
                each.sort_by_key(|p| p["id"].as_str().unwrap_or_default().to_string());
                (item["sequence_id"].as_i64().expect("a number"), each)
            })
            .collect()
    };
    let ours = people(&got.body["results"], false);
    let theirs = people(&ex["response"]["body"]["results"], true);
    assert!(theirs.values().any(|each| each.len() == 2), "the recorded list has an item with two people");
    assert_eq!(ours, theirs);
}

/// Each seeded project, as the owner's list shows it: what the seed gave it (its name, key, description, network, lead and default
/// state) and who is in it. The cycles and modules the oracle's project has are D-030 (1), so they are not compared.
#[test]
fn the_seeded_projects_are_listed_as_the_oracle_recorded() {
    let (router, manifest) = loaded();
    let ex = recorded("projects.json", "list");
    let path = real_path(&manifest, ex["request"]["path"].as_str().expect("a path"));
    let got = send_as(&router, token(&manifest, ex["request"]["auth"].as_str().expect("an auth")), "GET", &path);
    assert_eq!(got.status, 200);
    let by_key = |results: &Value| -> std::collections::BTreeMap<String, Value> {
        results
            .as_array()
            .expect("results")
            .iter()
            .map(|p| (p["identifier"].as_str().expect("a key").to_string(), p.clone()))
            .collect()
    };
    let (ours, theirs) = (by_key(&got.body["results"]), by_key(&ex["response"]["body"]["results"]));
    assert_eq!(ours.keys().collect::<Vec<_>>(), theirs.keys().collect::<Vec<_>>(), "both projects");
    for (key, recorded_project) in &theirs {
        for field in ["name", "identifier", "description", "network", "total_members", "is_member", "member_role"] {
            assert_eq!(ours[key][field], recorded_project[field], "{key}: {field}");
        }
        let id_of = |field: &str| recorded_project[field].as_str().map_or(Value::Null, |p| resolve(&manifest, p));
        assert_eq!(ours[key]["project_lead"], id_of("project_lead"), "{key}: project_lead");
        assert_eq!(ours[key]["default_state"], id_of("default_state"), "{key}: default_state");
    }
}

/// A project goes on from the number and the place its seeded items have: the first item the oracle created after its seed
/// is number 10, at 85535 (the largest place in Backlog was 75535). The recorded exchange is the answer.
#[test]
fn the_first_work_item_made_after_the_seed_is_numbered_and_placed_as_the_oracle_recorded() {
    let (router, manifest) = loaded();
    let ex = recorded("work_items.json", "create-minimal");
    let req = &ex["request"];
    let path = real_path(&manifest, req["path"].as_str().expect("a path"));
    let got = send_json(&router, token(&manifest, req["auth"].as_str().expect("an auth")), "POST", &path, &req["body"]);
    assert_eq!(i64::from(got.status), ex["response"]["status"].as_i64().expect("a status"));
    assert_eq!(got.body["sequence_id"], ex["response"]["body"]["sequence_id"], "the number");
    assert_eq!(got.body["sort_order"], ex["response"]["body"]["sort_order"], "the place, which is never normalised");
    assert_eq!(got.body["state"], manifest["states"]["alpha:Backlog"], "the project's default state");
    // and the list has the nine that were seeded and this one
    let list = send_as(
        &router,
        token(&manifest, "owner"),
        "GET",
        &real_path(&manifest, "/api/v1/workspaces/oracle-ws/projects/<project:alpha>/work-items/"),
    );
    assert_eq!(list.body["total_results"], json!(10));
}

/// What a project shows of its states and labels: five states (the triage one is hidden) and three labels, as recorded.
#[test]
fn the_states_and_labels_of_a_seeded_project_are_listed_as_the_oracle_recorded() {
    let (router, manifest) = loaded();
    let owner = token(&manifest, "owner");
    for (file, resource) in [("states.json", "states"), ("labels.json", "labels")] {
        let ex = recorded(file, "list");
        let path = real_path(&manifest, ex["request"]["path"].as_str().expect("a path"));
        assert_eq!(path.rsplit('/').nth(1), Some(resource));
        let got = send_as(&router, owner, "GET", &path);
        assert_eq!(got.status, 200, "{resource}");
        let names = |v: &Value| -> Vec<String> {
            v["results"]
                .as_array()
                .expect("results")
                .iter()
                .map(|r| r["name"].as_str().expect("a name").into())
                .collect()
        };
        assert_eq!(
            names(&got.body),
            names(&ex["response"]["body"]),
            "{resource}: the names, in the order of the answer"
        );
    }
}
