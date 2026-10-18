//! One axum router for both composition roots.
//!
//! The roots inject an [`App`] (and an [`ApiConfig`]) as request extensions:
//!   * native  -> `router().layer(Extension(app))`
//!   * Worker  -> `req.extensions_mut().insert(app)` once per request (adapters such as the
//!                Hyperdrive socket are request-scoped in workerd, so the `App` is too)
//!
//! Handlers therefore never see a platform type.

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Query};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use thrust_contracts::*;
use thrust_core::{App, CoreError};

/// Per-deployment HTTP-layer config (not business config; see `thrust_core::CoreConfig`).
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// Mounts `/internal/*` and `/_test/*`. Off in any real deployment.
    pub test_routes: bool,
    /// Shared secret for `/internal/*`. Supplied from the environment, never from a file.
    pub admin_token: Option<String>,
    /// Free-form label returned by `/_info` (e.g. "native" / "worker").
    pub target: String,
    pub adapters: String,
}

/// Response extension carrying the real cause of a 500 so a root-level layer can log it
/// without the body ever leaking it.
#[derive(Debug, Clone)]
pub struct InternalErrorDetail(pub String);

pub struct ApiError(pub CoreError);

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        ApiError(e)
    }
}

fn err_body(code: &str, message: String) -> Json<ApiErrorBody> {
    Json(ApiErrorBody { error: ApiErrorDetail { code: code.into(), message } })
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response<Body> {
        let (status, code, msg, detail) = match &self.0 {
            CoreError::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated", "missing or invalid credentials".to_string(), None),
            CoreError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "insufficient role".to_string(), None),
            CoreError::NotFound => (StatusCode::NOT_FOUND, "not_found", "not found".to_string(), None),
            CoreError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone(), None),
            CoreError::RateLimited { .. } => (StatusCode::TOO_MANY_REQUESTS, "rate_limited", "too many requests".to_string(), None),
            CoreError::Invalid(m) => (StatusCode::BAD_REQUEST, "invalid", m.clone(), None),
            CoreError::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "too_large", "payload too large".to_string(), None),
            CoreError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, "internal", "internal error".to_string(), Some(m.clone())),
        };
        let mut resp = (status, err_body(code, msg)).into_response();
        if let CoreError::RateLimited { retry_after_secs } = self.0 {
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                resp.headers_mut().insert(RETRY_AFTER, v);
            }
        }
        if let Some(d) = detail {
            resp.extensions_mut().insert(InternalErrorDetail(d));
        }
        resp
    }
}

type ApiResult<T> = Result<T, ApiError>;

fn missing_app() -> ApiError {
    ApiError(CoreError::Internal("App extension not installed by the composition root".into()))
}

/// `Authorization: Bearer <token>` -> `Principal` through the `SessionStore` port.
pub struct Authed(pub Principal);

impl<S: Send + Sync> FromRequestParts<S> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let app = parts.extensions.get::<App>().cloned().ok_or_else(missing_app)?;
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(ApiError(CoreError::Unauthenticated))?;
        let p = thrust_core::authenticate(&app, token).await?;
        Ok(Authed(p))
    }
}

/// Constant-time-ish comparison for the admin token.
fn token_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Gate for `/internal/*`: routes exist only when `test_routes` is on, and need the token.
pub struct Admin;

impl<S: Send + Sync> FromRequestParts<S> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let cfg = parts.extensions.get::<ApiConfig>().cloned().ok_or_else(missing_app)?;
        if !cfg.test_routes {
            return Err(ApiError(CoreError::NotFound));
        }
        let given = parts.headers.get("x-admin-token").and_then(|v| v.to_str().ok());
        match (cfg.admin_token.as_deref(), given) {
            (Some(want), Some(got)) if token_eq(want, got) => Ok(Admin),
            _ => Err(ApiError(CoreError::Unauthenticated)),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------------

async fn healthz() -> &'static str {
    "ok"
}

async fn info(Extension(cfg): Extension<ApiConfig>) -> ApiResult<Json<serde_json::Value>> {
    if !cfg.test_routes {
        return Err(ApiError(CoreError::NotFound));
    }
    Ok(Json(serde_json::json!({ "target": cfg.target, "adapters": cfg.adapters })))
}

async fn panic_route(Extension(cfg): Extension<ApiConfig>) -> ApiResult<&'static str> {
    if !cfg.test_routes {
        return Err(ApiError(CoreError::NotFound));
    }
    panic!("deliberate panic from /_test/panic");
}

async fn seed(_: Admin, Extension(app): Extension<App>, Json(f): Json<SeedFixture>) -> ApiResult<StatusCode> {
    app.ports.db.seed(&f).await.map_err(CoreError::from)?;
    for s in &f.sessions {
        app.ports.sessions.put(&s.token, &s.principal, 3600).await.map_err(CoreError::from)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn sweep(_: Admin, Extension(app): Extension<App>) -> ApiResult<Json<thrust_core::SweepReport>> {
    Ok(Json(thrust_core::sweep_outbox(&app, 100).await?))
}

async fn outbox_depth(_: Admin, Extension(app): Extension<App>) -> ApiResult<Json<serde_json::Value>> {
    let n = app.ports.db.outbox_depth().await.map_err(CoreError::from)?;
    Ok(Json(serde_json::json!({ "undispatched": n })))
}

async fn create_work_item(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path(project): Path<ProjectId>,
    Json(req): Json<CreateWorkItemRequest>,
) -> ApiResult<(StatusCode, Json<WorkItem>)> {
    let item = thrust_core::create_work_item(&app, &who, project, req).await?;
    Ok((StatusCode::CREATED, Json(item)))
}

#[derive(Deserialize)]
struct ListQuery {
    scope: Option<String>,
}

async fn list_work_items(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path(project): Path<ProjectId>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<WorkItem>>> {
    let vis = match q.scope.as_deref() {
        None | Some("visible") => Visibility::Visible,
        Some("with_deleted") => Visibility::WithDeleted,
        Some(other) => {
            return Err(ApiError(CoreError::Invalid(format!("unknown scope '{other}'"))));
        }
    };
    Ok(Json(thrust_core::list_work_items(&app, &who, project, vis).await?))
}

async fn delete_work_item(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((project, id)): Path<(ProjectId, WorkItemId)>,
) -> ApiResult<StatusCode> {
    thrust_core::delete_work_item(&app, &who, project, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_activity_for_item(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((project, id)): Path<(ProjectId, WorkItemId)>,
) -> ApiResult<Json<Vec<Activity>>> {
    Ok(Json(thrust_core::list_activity(&app, &who, project, Some(id)).await?))
}

async fn list_activity_for_project(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path(project): Path<ProjectId>,
) -> ApiResult<Json<Vec<Activity>>> {
    Ok(Json(thrust_core::list_activity(&app, &who, project, None).await?))
}

async fn put_attachment(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((project, id, name)): Path<(ProjectId, WorkItemId, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let ct = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
    thrust_core::put_attachment(&app, &who, project, id, &name, body.to_vec(), ct).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_attachment(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((project, id, name)): Path<(ProjectId, WorkItemId, String)>,
) -> ApiResult<Response<Body>> {
    let blob = thrust_core::get_attachment(&app, &who, project, id, &name).await?;
    let mut resp = Response::new(Body::from(blob.bytes));
    if let Some(ct) = blob.content_type.and_then(|c| HeaderValue::from_str(&c).ok()) {
        resp.headers_mut().insert(CONTENT_TYPE, ct);
    }
    Ok(resp)
}

async fn delete_attachment(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((project, id, name)): Path<(ProjectId, WorkItemId, String)>,
) -> ApiResult<StatusCode> {
    thrust_core::delete_attachment(&app, &who, project, id, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/_info", get(info))
        .route("/_test/panic", get(panic_route))
        .route("/internal/seed", post(seed))
        .route("/internal/sweep", post(sweep))
        .route("/internal/outbox-depth", get(outbox_depth))
        .route(
            "/api/v1/projects/{project_id}/work-items",
            post(create_work_item).get(list_work_items),
        )
        .route("/api/v1/projects/{project_id}/work-items/{id}", delete(delete_work_item))
        .route(
            "/api/v1/projects/{project_id}/work-items/{id}/activity",
            get(list_activity_for_item),
        )
        .route("/api/v1/projects/{project_id}/activity", get(list_activity_for_project))
        .route(
            "/api/v1/projects/{project_id}/work-items/{id}/attachments/{name}",
            put(put_attachment).get(get_attachment).delete(delete_attachment),
        )
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
}
