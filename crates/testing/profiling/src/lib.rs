//! In-process profiling workloads that span crates (ADR-0009, `crates/testing/profiling`). A workload drives the real code, here
//! the v1 router over `thrust-core` and the memory adapters, through `tower::ServiceExt::oneshot`, with no server and no network,
//! so a profiler sees the application's own work. It is deterministic: the world it runs against (`tests/common`) has a fixed
//! clock, and ids and randomness from counters. The world is built in `tests/`, not here, because it mints the system's proof
//! (`SystemScope::mint`), which only a composition root and a crate's own tests may do (scripts/check-deps.sh).
//!
//! The `prof_` tests (`tests/heap.rs`) profile it and write their profiles under `target/profiling/`, so they run only with the
//! `prof` profile. `tests/workload.rs` holds the workload itself to answering every request, in the default loop.

use axum::body::Body;
use axum::http::{Method, Request};
use axum::Router;
use serde_json::{json, Value};
use thrust_contracts::Uuid;
use tower::ServiceExt;

/// The world's workspace (its slug), its project, and its admin's API key, which every request carries.
pub const SLUG: &str = "prof";
pub const WORKSPACE: u128 = 0xA0;
pub const PROJECT: u128 = 0xA1;
pub const ADMIN: u128 = 0x10;
pub const KEY: &str = "key-prof-admin";

/// How many of each request the workload makes, in this order: creates, then lists, searches and updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mix {
    pub creates: usize,
    pub lists: usize,
    pub searches: usize,
    pub updates: usize,
}

impl Mix {
    /// What the `prof_` tests profile: two hundred creates, fifty lists, twenty searches and fifty updates.
    pub const PROFILE: Mix = Mix { creates: 200, lists: 50, searches: 20, updates: 50 };

    pub fn requests(&self) -> usize {
        self.creates + self.lists + self.searches + self.updates
    }
}

/// What a run did: how many requests it made, and each one not answered with a 2xx, with its status.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub requests: usize,
    pub refused: Vec<(String, u16)>,
}

/// Run `mix` against `router` (the v1 routes over a world with `SLUG`, `PROJECT` and `KEY`). `ballast` bytes are allocated, held
/// while the requests run, and freed at the end: zero for a real profile, a known amount for the control that shows the profile
/// counts what the run allocates.
pub fn run(router: &Router, mix: &Mix, ballast: usize) -> Outcome {
    let held = std::hint::black_box(vec![1u8; ballast]);
    let mut out = Outcome::default();
    let items = format!("/api/v1/workspaces/{SLUG}/projects/{}/work-items/", Uuid::from_u128(PROJECT));
    let mut ids = Vec::new();
    for i in 0..mix.creates {
        let (status, body) =
            send(router, &mut out, "POST", &items, Some(json!({ "name": format!("Profiled work item {i}") })));
        if (200..300).contains(&status) {
            if let Some(id) = body.get("id").and_then(Value::as_str) {
                ids.push(id.to_string());
            }
        }
    }
    for _ in 0..mix.lists {
        send(router, &mut out, "GET", &items, None);
    }
    for i in 0..mix.searches {
        let q = format!("/api/v1/workspaces/{SLUG}/work-items/search/?search=item%20{}", i % 10);
        send(router, &mut out, "GET", &q, None);
    }
    for i in 0..mix.updates {
        match ids.get(i % ids.len().max(1)) {
            Some(id) => {
                let one = format!("{items}{id}/");
                let priority = ["low", "medium", "high"][i % 3];
                send(router, &mut out, "PATCH", &one, Some(json!({ "priority": priority })));
            }
            None => {
                out.requests += 1;
                out.refused.push(("PATCH with no work item to update".into(), 0));
            }
        }
    }
    drop(held);
    out
}

/// One request, as the admin, through the router; records a refusal in `out` and returns the status and the body.
fn send(router: &Router, out: &mut Outcome, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    out.requests += 1;
    let mut req = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).expect("a method"))
        .uri(uri)
        .header("x-api-key", KEY);
    let bytes = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            serde_json::to_vec(&v).expect("a body")
        }
        None => Vec::new(),
    };
    let req = req.body(Body::from(bytes)).expect("a request");
    let (status, body) = futures_executor::block_on(async {
        let resp = router.clone().oneshot(req).await.expect("the router answers");
        let status = resp.status().as_u16();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.expect("a body");
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    });
    if !(200..300).contains(&status) {
        out.refused.push((format!("{method} {uri}"), status));
    }
    (status, body)
}
