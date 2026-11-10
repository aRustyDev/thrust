//! The fixture against what Plane recorded (`parity_`): the oracle's own `seed-manifest.json`, and the rows the golden corpus
//! shows for the seeded states, labels, projects and work items. Both are output of an independent oracle (the pinned Plane,
//! `tools/oracle-v1/`), so a static comparison is the right test here (AGENTS.md, testing rule 9 (c)).
//!
//! The corpus holds the response bodies after normalisation: an id is a placeholder (`<issue:i3>`, `<user:owner>`,
//! `<label:alpha:docs>`), a time is `<ts>`, a date `<date>`, and `null` stays `null`. A value that is set or not is compared as that.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use thrust_contracts::chrono::{DateTime, Utc};
use thrust_contracts::*;
use thrust_oracle_fixture::*;

/// Any instant will do: the rows compared here do not depend on it.
fn at() -> Timestamp {
    DateTime::<Utc>::from_timestamp(1_800_000_000, 0).expect("a valid instant")
}

fn oracle_file(rel: &str) -> Value {
    let path = format!("{}/../../tools/oracle-v1/{rel}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The `results` of the recorded exchange of that name in a corpus file.
fn recorded_results(file: &str, exchange: &str) -> Vec<Value> {
    let corpus = oracle_file(&format!("corpus/{file}"));
    let found = corpus["exchanges"]
        .as_array()
        .and_then(|all| all.iter().find(|e| e["name"] == exchange))
        .unwrap_or_else(|| panic!("{file} has no exchange named {exchange}"));
    found["response"]["body"]["results"]
        .as_array()
        .unwrap_or_else(|| panic!("{file}/{exchange} has no results"))
        .clone()
}

/// Every row of the tables this crate copies from `seed.py` is in `seed.py`, written the way it writes it: the people and their roles,
/// the projects, the labels, the work items (their states, priorities, people, labels, parents and offsets in days) and the tokens.
/// The corpus cannot show most of these (a date is only set or not, a role only for the caller), so the script itself is the oracle.
#[test]
fn parity_the_tables_are_the_oracles_seed_script() {
    let script =
        std::fs::read_to_string(format!("{}/../../tools/oracle-v1/scripts/seed.py", env!("CARGO_MANIFEST_DIR")))
            .expect("the vendored seed script");
    let role = |r: Option<Role>| match r {
        Some(Role::Admin) => "20",
        Some(Role::Member) => "15",
        Some(Role::Guest) => "5",
        None => "None",
    };
    let quoted = |names: &[&str]| names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
    let days = |d: Option<i64>| d.map_or("None".to_string(), |d| d.to_string());

    let mut expected: Vec<(String, String)> = Vec::new();
    for u in &USERS {
        let (key, email, first, last) = (u.key, u.email, u.first_name, u.last_name);
        let (ws, alpha) = (role(u.workspace_role), role(u.alpha_role));
        expected.push((
            format!("person {key}"),
            format!("\"{key}\": (\"{email}\", \"{first}\", \"{last}\", {ws}, {alpha}),"),
        ));
    }
    for (key, name, identifier, network, members) in PROJECTS {
        let network = match network {
            ProjectNetwork::Public => 2,
            ProjectNetwork::Secret => 0,
        };
        let members = quoted(members);
        expected.push((
            format!("project {key}"),
            format!("\"{key}\": (\"{name}\", \"{identifier}\", {network}, [{members}]),"),
        ));
    }
    let labels: Vec<String> = LABELS.iter().map(|(name, color)| format!("(\"{name}\", \"{color}\")")).collect();
    expected.push(("the labels".into(), format!("({})", labels.join(", "))));
    for it in ALPHA_ITEMS.iter().chain(BETA_ITEMS.iter()) {
        let parent = it.parent.map_or("None".to_string(), |p| format!("\"{p}\""));
        let (assignees, labels) = (quoted(it.assignees), quoted(it.labels));
        let (key, name, state, priority) = (it.key, it.name, it.state, it.priority.as_str());
        let (start, target) = (days(it.start), days(it.target));
        expected.push((
            format!("item {key}"),
            format!("(\"{key}\", \"{name}\", \"{state}\", \"{priority}\", [{assignees}], [{labels}], {parent}, {start}, {target}),"),
        ));
    }
    for (key, token, owner, expired, inactive) in TOKENS {
        if expired || inactive {
            // these two are written over several lines, with what makes them refused on the last
            expected.push((format!("token {key}"), format!("\"{token}\",")));
            let extra = if expired { "{\"expired_at\": now - timedelta(days=1)}" } else { "{\"is_active\": False}" };
            expected.push((format!("what refuses {key}"), extra.to_string()));
        } else {
            expected.push((format!("token {key}"), format!("\"{key}\": (\"{token}\", \"{owner}\", {{}}),")));
        }
    }
    for (what, line) in &expected {
        assert!(script.contains(line), "seed.py has no {what}: expected the text `{line}`");
    }
    // a pass that checked nothing is not a pass: 4 people, 2 projects, the labels, 11 items, 7 tokens and 2 refusals
    assert_eq!(expected.len(), 4 + 2 + 1 + 11 + 7 + 2);
}

/// Who is in which project, and as what: the oracle's Alpha has its owner, member and guest as admin, member and guest, and its
/// secret Beta has only the owner (`seed.py`: `member_keys`, and `USERS[mk][4] if key == "alpha" else 20`).
#[test]
fn parity_the_projects_have_the_members_and_roles_of_the_seed_script() {
    let f = oracle_seed(at()).fixture;
    let roles_in = |project: &str| -> BTreeMap<Uuid, Role> {
        let id = uid(&format!("project:{project}"));
        f.memberships.iter().filter(|m| m.project_id.0 == id).map(|m| (m.user_id.0, m.role)).collect()
    };
    let user = |key: &str| uid(&format!("user:{key}"));
    assert_eq!(
        roles_in("alpha"),
        BTreeMap::from([(user("owner"), Role::Admin), (user("member"), Role::Member), (user("guest"), Role::Guest)])
    );
    assert_eq!(roles_in("beta"), BTreeMap::from([(user("owner"), Role::Admin)]));
    // and the workspace: the owner, member and guest, with the same roles; the outsider is in none
    let ws: BTreeMap<Uuid, Role> = f.workspace_roles.iter().map(|r| (r.user_id.0, r.role)).collect();
    assert_eq!(
        ws,
        BTreeMap::from([(user("owner"), Role::Admin), (user("member"), Role::Member), (user("guest"), Role::Guest)])
    );
}

#[test]
fn parity_the_manifest_is_the_oracles_manifest() {
    let committed = oracle_file("corpus/seed-manifest.json");
    let ours = oracle_seed(at()).manifest;
    // the keys that are reproduced (the module's note says why the others are not)
    for key in ["workspace", "users", "projects", "states", "labels", "issues", "tokens"] {
        assert_eq!(ours[key], committed[key], "the manifest's `{key}` is not the oracle's");
    }
}

/// The placeholder the corpus writes for an id of the fixture, made from the manifest the oracle committed (not ours).
fn placeholders() -> BTreeMap<String, String> {
    let committed = oracle_file("corpus/seed-manifest.json");
    let mut by_id = BTreeMap::new();
    let mut put = |id: &Value, name: String| {
        by_id.insert(id.as_str().expect("an id is text").to_string(), name);
    };
    for (key, v) in committed["users"].as_object().expect("users") {
        put(&v["id"], format!("<user:{key}>"));
    }
    for (key, v) in committed["issues"].as_object().expect("issues") {
        put(&v["id"], format!("<issue:{key}>"));
    }
    for (key, v) in committed["labels"].as_object().expect("labels") {
        put(v, format!("<label:{key}>"));
    }
    for (key, v) in committed["states"].as_object().expect("states") {
        put(v, format!("<state:{key}>"));
    }
    for (key, v) in committed["projects"].as_object().expect("projects") {
        put(&v["id"], format!("<project:{key}>"));
    }
    by_id
}

#[test]
fn parity_the_visible_states_are_the_recorded_ones() {
    let f = oracle_seed(at()).fixture;
    let ids = placeholders();
    // the corpus lists the states of Alpha only
    let project = ids.iter().find(|(_, v)| v.as_str() == "<project:alpha>").map(|(k, _)| k.clone()).unwrap();
    let mut ours: Vec<(String, String, f64, String, bool)> = f
        .states
        .iter()
        .filter(|s| s.project_id.0.to_string() == project && s.group != StateGroup::Triage)
        .map(|s| (s.name.clone(), s.color.clone(), s.sequence, s.group.as_str().to_string(), s.is_default))
        .collect();
    let recorded: Vec<(String, String, f64, String, bool)> = recorded_results("states.json", "list")
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap().to_string(),
                r["color"].as_str().unwrap().to_string(),
                r["sequence"].as_f64().unwrap(),
                r["group"].as_str().unwrap().to_string(),
                r["default"].as_bool().unwrap(),
            )
        })
        .collect();
    // the corpus lists by sequence; the triage state is the one the pin's default manager hides
    ours.sort_by(|a, b| a.2.total_cmp(&b.2));
    assert_eq!(ours, recorded, "the visible states of Alpha");
}

#[test]
fn parity_the_triage_state_is_the_one_the_corpus_cannot_show() {
    // Derived, not recorded (the module's note): saved last, so `largest + 15000` over the five that are visible.
    let f = oracle_seed(at()).fixture;
    for project in &f.projects {
        let of_project: Vec<&SeedState> = f.states.iter().filter(|s| s.project_id == project.id).collect();
        let visible_largest =
            of_project.iter().filter(|s| s.group != StateGroup::Triage).map(|s| s.sequence).fold(0.0, f64::max);
        let triage: Vec<&&SeedState> = of_project.iter().filter(|s| s.group == StateGroup::Triage).collect();
        assert_eq!(triage.len(), 1, "one triage state in {}", project.name);
        assert_eq!(triage[0].sequence, visible_largest + 15_000.0);
    }
}

#[test]
fn parity_the_labels_are_the_recorded_ones() {
    let f = oracle_seed(at()).fixture;
    let alpha = f.projects.iter().find(|p| p.name == "Oracle Alpha").unwrap().id;
    let mut ours: Vec<(String, String, f64)> = f
        .labels
        .iter()
        .filter(|l| l.project_id == alpha)
        .map(|l| (l.name.clone(), l.color.clone(), l.sort_order))
        .collect();
    let mut recorded: Vec<(String, String, f64)> = recorded_results("labels.json", "list")
        .iter()
        .map(|r| {
            (r["name"].as_str().unwrap().into(), r["color"].as_str().unwrap().into(), r["sort_order"].as_f64().unwrap())
        })
        .collect();
    // the corpus lists newest first; what is compared is the rows
    ours.sort_by(|a, b| a.2.total_cmp(&b.2));
    recorded.sort_by(|a, b| a.2.total_cmp(&b.2));
    assert_eq!(ours, recorded);
}

#[test]
fn parity_the_projects_are_the_recorded_ones() {
    let f = oracle_seed(at()).fixture;
    let ids = placeholders();
    let recorded = recorded_results("projects.json", "list");
    assert_eq!(recorded.len(), 2, "the owner sees both seeded projects");
    for r in recorded {
        let identifier = r["identifier"].as_str().unwrap();
        let details = f.project_details.iter().find(|d| d.identifier.as_deref() == Some(identifier)).unwrap();
        let project = f.projects.iter().find(|p| p.id == details.project_id).unwrap();
        assert_eq!(project.name, r["name"].as_str().unwrap(), "{identifier}: name");
        assert_eq!(details.description, r["description"].as_str().unwrap(), "{identifier}: description");
        // Plane's `network`: 0 is secret, 2 is public
        let network = match details.network {
            ProjectNetwork::Secret => 0,
            ProjectNetwork::Public => 2,
        };
        assert_eq!(network, r["network"].as_i64().unwrap(), "{identifier}: network");
        let lead = details.project_lead.map(|u| ids[&u.0.to_string()].clone());
        assert_eq!(lead.as_deref(), r["project_lead"].as_str(), "{identifier}: lead");
        let default_state = details.default_state.map(|s| ids[&s.to_string()].clone());
        assert_eq!(default_state.as_deref(), r["default_state"].as_str(), "{identifier}: default_state");
    }
}

#[test]
fn parity_the_people_are_the_recorded_ones() {
    // an expanded assignee is a user as the pin shows one: who they are, and what they are called
    let f = oracle_seed(at()).fixture;
    let ids = placeholders();
    let mut checked = BTreeSet::new();
    for r in recorded_results("work_items.json", "list-expand-assignees-labels-state") {
        for person in r["assignees"].as_array().unwrap() {
            let placeholder = person["id"].as_str().unwrap();
            let user = f
                .users
                .iter()
                .find(|u| ids[&u.0.to_string()] == placeholder)
                .unwrap_or_else(|| panic!("{placeholder}"));
            let profile = f.profiles.iter().find(|p| p.user_id == *user).expect("a profile");
            let login = f.logins.iter().find(|l| l.user_id == *user).expect("a login");
            assert_eq!(profile.first_name, person["first_name"].as_str().unwrap(), "{placeholder}: first_name");
            assert_eq!(profile.last_name, person["last_name"].as_str().unwrap(), "{placeholder}: last_name");
            assert_eq!(profile.display_name, person["display_name"].as_str().unwrap(), "{placeholder}: display_name");
            assert_eq!(login.email, person["email"].as_str().unwrap(), "{placeholder}: email");
            checked.insert(placeholder.to_string());
        }
    }
    // a pass that compared no person is not a pass: the corpus names both assignees of the seed's items
    assert_eq!(checked, BTreeSet::from(["<user:member>".to_string(), "<user:owner>".to_string()]));
}

#[test]
fn parity_the_work_items_are_the_recorded_ones() {
    let f = oracle_seed(at()).fixture;
    let ids = placeholders();
    let alpha = f.projects.iter().find(|p| p.name == "Oracle Alpha").unwrap().id;
    let name_of = |id: Uuid| ids[&id.to_string()].clone();
    let recorded = recorded_results("work_items.json", "list");
    assert_eq!(recorded.len(), 9, "the nine items of Alpha");
    for r in recorded {
        let sequence = r["sequence_id"].as_i64().unwrap();
        let ours = f
            .work_items
            .iter()
            .find(|w| w.project_id == alpha && w.sequence_id == sequence)
            .unwrap_or_else(|| panic!("no item {sequence}"));
        let which = format!("item {sequence} ({})", ours.name);
        assert_eq!(ours.name, r["name"].as_str().unwrap(), "{which}: name");
        assert_eq!(ours.priority.as_str(), r["priority"].as_str().unwrap(), "{which}: priority");
        assert_eq!(ours.sort_order as f64, r["sort_order"].as_f64().unwrap(), "{which}: sort_order (never normalised)");
        assert_eq!(ours.description_html, r["description_html"].as_str().unwrap(), "{which}: description_html");
        assert_eq!(ours.state_id.map(name_of), r["state"].as_str().map(str::to_string), "{which}: state");
        assert_eq!(ours.parent_id.map(name_of), r["parent"].as_str().map(str::to_string), "{which}: parent");
        assert_eq!(ours.created_by, None, "{which}: created_by");
        assert!(r["created_by"].is_null(), "{which}: the oracle's seeded items have no maker");
        // a time or a date is compared as set or not: the normaliser replaced the value
        assert_eq!(ours.completed_at.is_some(), !r["completed_at"].is_null(), "{which}: completed_at");
        assert_eq!(ours.start_date.is_some(), !r["start_date"].is_null(), "{which}: start_date");
        assert_eq!(ours.target_date.is_some(), !r["target_date"].is_null(), "{which}: target_date");
        // who and what a work item has: the corpus lists them in Plane's order, which is the next slice's concern, so as sets
        let people: BTreeSet<String> = ours.assignee_ids.iter().map(|u| name_of(u.0)).collect();
        let recorded_people: BTreeSet<String> =
            r["assignees"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!(people, recorded_people, "{which}: assignees");
        let labels: BTreeSet<String> = ours.label_ids.iter().map(|l| name_of(*l)).collect();
        let recorded_labels: BTreeSet<String> =
            r["labels"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!(labels, recorded_labels, "{which}: labels");
    }
}
