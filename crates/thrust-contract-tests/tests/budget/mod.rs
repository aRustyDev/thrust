//! The round-trip budget (issue #39, FR-08 item 2): a list request sends at most 5 statements to the database.
//!
//! Plane's project page fires 13 reads and its workspace wrapper 9 (`research/0003`), and each statement is a network round
//! trip from the Worker to the database (about 1 to 3 ms when the Worker is placed beside it, 20 to 30 ms when it is not). A
//! compat layer that copies Django's query shapes would spend that on every list, so the budget is a gate and not a hope.
//!
//! The target says how many statements a request sent in `x-thrust-db-round-trips`, and only when test routes are on and
//! the adapter set counts at all (Postgres: `native-pg` and `worker-cf`). The `memory` targets send no header, and the test
//! says so: it asserts the header is absent there, so it cannot pass on a target that counts nothing by never looking.
//! Both outcomes name their cause in the failure ("over budget", "below floor"), because `scripts/mutants.sh` tells a mutant
//! that was caught from a run that failed for another reason by that text.
//!
//! What is measured is a steady-state request, after the first of each. The first use of an API key stamps `last_used_at` in
//! the background (`thrust-core/src/auth.rs`), and it runs on the target's own schedule, so it is waited for and not counted.
//!
//! The lists are the six of FR-08 and #58, in the shape a client asks for them by default: the slice's work items, Plane's
//! projects (S17) and work items (S47) over the internal API, and `/api/v1`'s projects, states, labels and work items. Out of
//! scope on purpose, and a known gap (`docs/src/testing/status.md`): `/api/v1` work items with `?expand=assignees`, which
//! sends two statements per distinct assignee.

use std::time::Duration;

use serde_json::Value;
use thrust_contract_tests::*;
use thrust_contracts::{
    ApiKeyInfo, ProjectId, Role, SeedApiKey, SeedFixture, SeedLabel, SeedMembership, SeedProject, SeedState,
    SeedWorkspace, StateGroup, WorkspaceSlug,
};
use uuid::Uuid;

use super::t;

/// Must equal `thrust_api::DB_ROUND_TRIPS_HEADER` (this crate is black-box and does not depend on it).
const HEADER: &str = "x-thrust-db-round-trips";

/// The most statements a list may send.
const BUDGET: u32 = 5;

/// The fewest a list can honestly send: it authenticates, checks the caller's membership and reads. A count below this means
/// the counter misses statements, which is how a counter that counts nothing would otherwise pass.
const FLOOR: u32 = 3;

/// Rows in every list. Over a floor of 3, a statement per row is over the budget of 5 from the third row; this leaves margin.
const ROWS: usize = 6;

/// One list: where it is, and who asks (an API key for `/api/v1`, a session for the rest).
struct Route {
    name: &'static str,
    path: String,
    api_key: bool,
}

fn routes(f: &Fixture) -> Vec<Route> {
    let (slug, project) = (&f.a.slug, f.a.project.0);
    let v1 = format!("/api/v1/workspaces/{slug}");
    let compat = format!("/api/workspaces/{slug}");
    vec![
        Route { name: "slice work items", path: wi_path(&f.a), api_key: false },
        Route { name: "compat projects (S17)", path: format!("{compat}/projects/"), api_key: false },
        Route { name: "compat issues (S47)", path: format!("{compat}/projects/{project}/issues/"), api_key: false },
        Route { name: "v1 projects", path: format!("{v1}/projects/"), api_key: true },
        Route { name: "v1 states", path: format!("{v1}/projects/{project}/states/"), api_key: true },
        Route { name: "v1 labels", path: format!("{v1}/projects/{project}/labels/"), api_key: true },
        Route { name: "v1 work items", path: format!("{v1}/projects/{project}/work-items/"), api_key: true },
    ]
}

/// Rows, states, labels, projects and one API key: every list has `ROWS` or more, and the key is the admin's.
async fn seed(t: &Target, f: &Fixture, key: &str) {
    let items: Vec<(String, Option<String>)> = (0..ROWS).map(|n| (format!("budget item {n}"), None)).collect();
    seed_work_items(t, f, &f.a, f.admin_a.id, &items).await;

    // The caller is in every extra project, and each one's identifier comes from its random id, so they cannot collide.
    let extra: Vec<SeedProject> = (0..ROWS)
        .map(|n| SeedProject {
            id: ProjectId(Uuid::new_v4()),
            workspace_id: f.ws_a,
            name: format!("budget project {n}"),
        })
        .collect();
    let memberships = extra
        .iter()
        .map(|p| SeedMembership { project_id: p.id, workspace_id: f.ws_a, user_id: f.admin_a.id, role: Role::Admin })
        .collect();
    let states = (0..ROWS)
        .map(|n| SeedState {
            id: Uuid::new_v4(),
            project_id: f.a.project,
            workspace_id: f.ws_a,
            name: format!("budget state {n}"),
            color: "#3f76ff".into(),
            sequence: 1000.0 * (n as f64 + 1.0),
            group: StateGroup::Started,
            is_default: false,
        })
        .collect();
    let labels = (0..ROWS)
        .map(|n| SeedLabel {
            id: Uuid::new_v4(),
            project_id: f.a.project,
            workspace_id: f.ws_a,
            name: format!("budget label {n}"),
            color: "#ff6b00".into(),
            parent_id: None,
            sort_order: 1000.0 * (n as f64 + 1.0),
        })
        .collect();
    let fixture = SeedFixture {
        workspaces: vec![SeedWorkspace { id: f.ws_a, slug: WorkspaceSlug::new(&f.a.slug) }],
        projects: extra,
        memberships,
        states,
        labels,
        api_keys: vec![SeedApiKey {
            token: key.to_string(),
            user_id: f.admin_a.id,
            label: "budget".into(),
            expires_at: None,
            revoked: false,
        }],
        ..SeedFixture::default()
    };
    let r = t.admin_post("/internal/seed", &serde_json::to_value(&fixture).unwrap()).await;
    assert_eq!(r.status(), 204, "seed the budget fixture: {:?}", r.text().await);
}

/// The first use of a key stamps `last_used_at` in the background. Use it once, and wait until the stamp is there, so that no
/// measured request starts that task.
async fn warm_the_key(t: &Target, f: &Fixture, key: &str, first: &Route) {
    let r = t.http.get(t.url(&first.path)).header("x-api-key", key).send().await.unwrap();
    assert_eq!(r.status(), 200, "the first use of the key: {:?}", r.text().await);
    eventually("the key's first use to be recorded", Duration::from_secs(15), || async {
        let r =
            t.http.get(t.url(&format!("{SLICE_PREFIX}/api-keys"))).bearer_auth(&f.admin_a.token).send().await.unwrap();
        let keys: Vec<ApiKeyInfo> = r.json().await.unwrap();
        keys.into_iter().find(|k| k.label == "budget" && k.last_used_at.is_some())
    })
    .await;
}

/// How many rows a list answered with: an array, or the `results` of a paginated object.
fn rows(body: &Value) -> usize {
    match body {
        Value::Array(a) => a.len(),
        Value::Object(o) => o.get("results").and_then(Value::as_array).map_or(0, Vec::len),
        _ => 0,
    }
}

/// One request of a list: the statements the target says it sent (None when it sent no header) and the rows it answered with.
async fn ask(t: &Target, f: &Fixture, key: &str, route: &Route) -> (Option<u32>, usize) {
    let req = t.http.get(t.url(&route.path));
    let req = if route.api_key { req.header("x-api-key", key) } else { req.bearer_auth(&f.admin_a.token) };
    let r = req.send().await.unwrap();
    let status = r.status();
    let sent = r.headers().get(HEADER).map(|v| {
        v.to_str()
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or_else(|| panic!("{}: the {HEADER} header is not a number: {v:?}", route.name))
    });
    let body: Value = r.json().await.unwrap_or(Value::Null);
    assert_eq!(status, 200, "{}: {body}", route.name);
    (sent, rows(&body))
}

#[tokio::test]
async fn list_endpoints_stay_within_five_db_round_trips() {
    let t = t();
    let adapters = t.info().await["adapters"].as_str().expect("/_info names the adapter set").to_string();
    let f = Fixture::seed(&t).await;
    let key = format!("thrk_budget-{}", Uuid::new_v4().simple());
    seed(&t, &f, &key).await;
    let routes = routes(&f);
    warm_the_key(&t, &f, &key, &routes[3]).await;

    let mut measured = Vec::new();
    for route in &routes {
        // the first request of a list may carry a one-off cost of its own; the budget is for the one after it
        ask(&t, &f, &key, route).await;
        let (sent, got) = ask(&t, &f, &key, route).await;
        // a list over too few rows cannot show a statement per row, so a thin list is a failure of the test and not a pass
        assert!(
            got >= ROWS,
            "{}: the list has {got} rows and the test seeded {ROWS}: it would not show a statement per row",
            route.name
        );
        measured.push((route.name, sent));
    }
    let report = measured.iter().map(|(name, n)| format!("{name}: {n:?}")).collect::<Vec<_>>().join(", ");
    // The numbers go in the log (the `budget` pass shows test output), so a green run says what each list costs and not only
    // that it fits.
    println!("[{adapters}] statements per list: {report}");

    match adapters.as_str() {
        "postgres" | "cloudflare" => {
            for (name, sent) in &measured {
                let n = sent.unwrap_or_else(|| {
                    panic!("{name} sent no {HEADER} header on a {adapters} target, which must count with test routes on ({report})")
                });
                assert!(n <= BUDGET, "{name} is over budget: {n} statements, at most {BUDGET} are allowed ({report})");
                assert!(
                    n >= FLOOR,
                    "{name} is below floor: {n} statements, and a list that authenticates, checks membership and reads sends at least {FLOOR}, so the counter misses some ({report})"
                );
            }
        }
        "memory" => {
            for (name, sent) in &measured {
                assert!(
                    sent.is_none(),
                    "{name} sent {HEADER}={sent:?} on a memory target, which counts nothing: a count there is not a count ({report})"
                );
            }
        }
        other => panic!("the budget test does not know the adapter set '{other}': say what it counts ({report})"),
    }
}
