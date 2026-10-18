// TEMPLATE (rust-test-logic), adapted from rust-testing-plan.zip, 2026-10-18: copy to tests/model_api.rs; your_api, router, AppState, /api/items and the id rules are EXAMPLES: substitute your crate, routes and documented behaviour (dev-dependencies: tower with `util`, serde_json, tokio with `rt`, tests/common/mod.rs).
//! Model-based API test: random sequences of HTTP calls against the real router,
//! checked step by step against an in-memory model of the documented behaviour.

use your_api::{router, AppState};
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tower::ServiceExt; // for `oneshot`

mod common;

#[derive(Clone, Debug)]
enum Call {
    Create(String),
    Get(usize),
    Delete(usize),
}

fn call() -> impl Strategy<Value = Call> {
    prop_oneof![
        "[a-z]{1,6}".prop_map(Call::Create),
        any::<usize>().prop_map(Call::Get),
        any::<usize>().prop_map(Call::Delete),
    ]
}

/// The spec: ids start at 1, increase monotonically, and are never reused.
struct Model {
    items: BTreeMap<u32, String>,
    next_id: u32,
    issued: Vec<u32>,
}

/// Mostly target issued ids (live or deleted); sometimes one that never existed.
fn pick(issued: &[u32], i: usize) -> u32 {
    if issued.is_empty() || i % 5 == 0 {
        9_999
    } else {
        issued[i % issued.len()]
    }
}

async fn send(app: &axum::Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Option<Value>) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).ok())
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn model_items_api_matches_model(calls in prop::collection::vec(call(), 1..30)) {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let app = router(AppState::default());
            let mut model = Model { items: BTreeMap::new(), next_id: 1, issued: vec![] };
            for call in calls {
                match call {
                    Call::Create(name) => {
                        let (status, body) = send(&app, Method::POST, "/api/items", Some(json!({ "name": name }))).await;
                        let id = model.next_id;
                        model.next_id += 1;
                        model.items.insert(id, name.clone());
                        model.issued.push(id);
                        prop_assert_eq!(status, StatusCode::CREATED);
                        prop_assert_eq!(body, Some(json!({ "id": id, "name": name })));
                    }
                    Call::Get(i) => {
                        let id = pick(&model.issued, i);
                        let (status, body) = send(&app, Method::GET, &format!("/api/items/{id}"), None).await;
                        match model.items.get(&id) {
                            Some(name) => {
                                prop_assert_eq!(status, StatusCode::OK);
                                prop_assert_eq!(body, Some(json!({ "id": id, "name": name })));
                            }
                            None => prop_assert_eq!(status, StatusCode::NOT_FOUND),
                        }
                    }
                    Call::Delete(i) => {
                        let id = pick(&model.issued, i);
                        let (status, _) = send(&app, Method::DELETE, &format!("/api/items/{id}"), None).await;
                        let expected = match model.items.remove(&id) {
                            Some(_) => StatusCode::NO_CONTENT,
                            None => StatusCode::NOT_FOUND,
                        };
                        prop_assert_eq!(status, expected);
                    }
                }
            }
            Ok::<(), TestCaseError>(())
        })?;
    }
}
