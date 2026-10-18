// Template: the api crate's src/lib.rs (router and its tests). Example names: Item, AppState; rename to yours. [re-run]
//! Minimal API used to validate the router-level and socket-level test patterns.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[derive(Clone, Default)]
pub struct AppState {
    pub items: Arc<RwLock<BTreeMap<u32, String>>>,
    last_id: Arc<std::sync::atomic::AtomicU32>,
}

#[derive(Deserialize)]
pub struct NewItem {
    pub name: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Item {
    pub id: u32,
    pub name: String,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/items", post(create_item))
        .route("/api/items/{id}", get(get_item).delete(delete_item))
        .with_state(state)
}

async fn create_item(
    State(state): State<AppState>,
    Json(new): Json<NewItem>,
) -> (StatusCode, Json<Item>) {
    // Ids are monotonic and never reused, even after deletes.
    let id = state.last_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    state.items.write().unwrap().insert(id, new.name.clone());
    (StatusCode::CREATED, Json(Item { id, name: new.name }))
}

async fn delete_item(State(state): State<AppState>, Path(id): Path<u32>) -> StatusCode {
    match state.items.write().unwrap().remove(&id) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::NOT_FOUND,
    }
}

async fn get_item(
    State(state): State<AppState>,
    Path(id): Path<u32>,
) -> Result<Json<Item>, StatusCode> {
    let items = state.items.read().unwrap();
    items
        .get(&id)
        .map(|name| Json(Item { id, name: name.clone() }))
        .ok_or(StatusCode::NOT_FOUND)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for `oneshot`

    /// Router-level request: no socket, no port, runs in microseconds.
    async fn get(app: Router, uri: &str) -> (StatusCode, String) {
        let resp = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn get_item_returns_json() {
        let state = AppState::default();
        state.items.write().unwrap().insert(7, "widget".into());
        let (status, body) = get(router(state), "/api/items/7").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"id":7,"name":"widget"}"#);
    }

    // Parameterized async cases: rstest generates one #[tokio::test] per case.
    #[rstest::rstest]
    #[case::existing(7, StatusCode::OK)]
    #[case::missing(8, StatusCode::NOT_FOUND)]
    #[tokio::test]
    async fn get_item_status(#[case] id: u32, #[case] expected: StatusCode) {
        let state = AppState::default();
        state.items.write().unwrap().insert(7, "widget".into());
        let (status, _) = get(router(state), &format!("/api/items/{id}")).await;
        assert_eq!(status, expected);
    }

    // Matrix: every value runs as its own test.
    #[rstest::rstest]
    #[tokio::test]
    async fn unknown_ids_are_404(#[values(0, 2, u32::MAX)] id: u32) {
        let (status, _) = get(router(AppState::default()), &format!("/api/items/{id}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // Fixtures: injected by argument name; each test gets a fresh value.
    #[rstest::fixture]
    fn seeded() -> AppState {
        let state = AppState::default();
        state.items.write().unwrap().insert(7, "widget".into());
        state
    }

    #[rstest::rstest]
    #[case::exact(7, StatusCode::OK)]
    #[case::neighbour(6, StatusCode::NOT_FOUND)]
    #[tokio::test]
    async fn seeded_lookup(seeded: AppState, #[case] id: u32, #[case] expected: StatusCode) {
        let (status, _) = get(router(seeded), &format!("/api/items/{id}")).await;
        assert_eq!(status, expected);
    }

    #[tokio::test]
    async fn missing_item_is_404() {
        let (status, _) = get(router(AppState::default()), "/api/items/1").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
