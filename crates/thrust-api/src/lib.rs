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
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, COOKIE, RETRY_AFTER, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, Response, StatusCode};
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
            CoreError::Csrf => (StatusCode::FORBIDDEN, "csrf_failed", "csrf token missing or wrong".to_string(), None),
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

// ---------------------------------------------------------------------------------------------
// Credentials: headers in, `Principal` out. The rules (which credential counts, when a mutation needs the CSRF token)
// are core's (`thrust_core::authenticate_request`); this is only the spelling of them on the wire.
// ---------------------------------------------------------------------------------------------

/// The session cookie. The `__Host-` prefix makes a browser accept it only with `Secure`, `Path=/` and no `Domain`, so
/// a sibling subdomain cannot plant one (ADR-0052 S1).
pub const SESSION_COOKIE: &str = "__Host-thrust_session";
/// The double-submit CSRF cookie, and the header a client sends the same value in (the name Django and Plane use).
pub const CSRF_COOKIE: &str = "__Host-thrust_csrf";
pub const CSRF_HEADER: &str = "x-csrftoken";

/// The value of cookie `name` in the request's `Cookie` headers (the first one, if a client sent it twice).
fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v)
}

fn credentials(parts: &Parts) -> thrust_core::Credentials<'_> {
    let h = &parts.headers;
    thrust_core::Credentials {
        // an `Authorization` header that is not a bearer token is not a credential here
        bearer: h.get(AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")),
        session_cookie: cookie_value(h, SESSION_COOKIE),
        csrf_cookie: cookie_value(h, CSRF_COOKIE),
        csrf_header: h.get(CSRF_HEADER).and_then(|v| v.to_str().ok()),
        mutating: !matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE),
    }
}

async fn authenticate_parts(parts: &Parts) -> Result<thrust_core::Authenticated, ApiError> {
    let app = parts.extensions.get::<App>().cloned().ok_or_else(missing_app)?;
    Ok(thrust_core::authenticate_request(&app, &credentials(parts)).await?)
}

/// The authenticated caller: a bearer token or the session cookie (a mutation made with the cookie also needs the CSRF
/// token), resolved from the session row in the database.
pub struct Authed(pub Principal);

impl<S: Send + Sync> FromRequestParts<S> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Authed(authenticate_parts(parts).await?.principal))
    }
}

/// The authenticated caller and how they proved it.
pub struct AuthedVia(pub thrust_core::Authenticated);

impl<S: Send + Sync> FromRequestParts<S> for AuthedVia {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(AuthedVia(authenticate_parts(parts).await?))
    }
}

/// Comparison for the admin token, in constant time.
fn token_eq(a: &str, b: &str) -> bool {
    thrust_core::ct_eq(a.as_bytes(), b.as_bytes())
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
    thrust_core::seed(&app, &f).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct RevokeSession {
    token: String,
}

/// Test support until sign-out exists (#38): revoke the session behind a bearer token. Idempotent, like the use
/// case: a token that is unknown or already revoked is a 204 too.
async fn revoke_session(_: Admin, Extension(app): Extension<App>, Json(b): Json<RevokeSession>) -> ApiResult<StatusCode> {
    thrust_core::revoke_session(&app, &b.token).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SetUserActive {
    user_id: UserId,
    active: bool,
}

/// Test support until deactivation exists (ADR-0053): deactivate or reactivate a user. 404 for a user that does not
/// exist, so a typo in a test is not a silent success.
async fn set_user_active(_: Admin, Extension(app): Extension<App>, Json(b): Json<SetUserActive>) -> ApiResult<StatusCode> {
    if thrust_core::set_user_active(&app, b.user_id, b.active).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError(CoreError::NotFound))
    }
}

// ---------------------------------------------------------------------------------------------
// Authentication routes (issue #38). BetterAuth's HTTP shapes in spirit, Plane's `get-csrf-token` as JSON.
// ---------------------------------------------------------------------------------------------

/// `Max-Age` for both cookies: the session's lifetime.
fn set_cookie(name: &str, value: &str, max_age_secs: i64) -> String {
    // Secure + __Host- (so Path=/ and no Domain), HttpOnly (a script cannot read the session or, as Plane does, the CSRF
    // cookie: the token is handed out as JSON instead), SameSite=Lax (the OIDC callback is a top-level navigation that
    // must carry the cookie to the next page, which Strict would drop).
    format!("{name}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}")
}

fn add_cookies(resp: &mut Response<Body>, cookies: &[String]) {
    for c in cookies {
        if let Ok(v) = HeaderValue::from_str(c) {
            resp.headers_mut().append(SET_COOKIE, v);
        }
    }
    // an answer that sets or clears a credential is never cached
    resp.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

fn session_cookies(token: &str, csrf: &str, max_age_secs: i64) -> [String; 2] {
    [set_cookie(SESSION_COOKIE, token, max_age_secs), set_cookie(CSRF_COOKIE, csrf, max_age_secs)]
}

fn cleared_cookies() -> [String; 2] {
    [set_cookie(SESSION_COOKIE, "", 0), set_cookie(CSRF_COOKIE, "", 0)]
}

async fn sign_in(Extension(app): Extension<App>, Json(req): Json<SignInRequest>) -> ApiResult<Response<Body>> {
    let s = thrust_core::sign_in(&app, &req.email, &req.password).await?;
    let body = SignInResponse { user_id: s.user_id, csrf_token: s.csrf_token.clone(), expires_at: s.expires_at };
    let mut resp = Json(body).into_response();
    add_cookies(&mut resp, &session_cookies(&s.token, &s.csrf_token, app.cfg.session_ttl_secs));
    Ok(resp)
}

/// A session credential only: the cookie (with the CSRF token) or the bearer token. Signing out revokes that one session.
async fn sign_out(AuthedVia(a): AuthedVia, Extension(app): Extension<App>) -> ApiResult<Response<Body>> {
    let thrust_core::Via::Session(hash) = a.via;
    thrust_core::sign_out(&app, &hash).await?;
    let mut resp = StatusCode::NO_CONTENT.into_response();
    add_cookies(&mut resp, &cleared_cookies());
    Ok(resp)
}

async fn sign_out_all(AuthedVia(a): AuthedVia, Extension(app): Extension<App>) -> ApiResult<Response<Body>> {
    let n = thrust_core::sign_out_everywhere(&app, &a.principal).await?;
    let mut resp = Json(serde_json::json!({ "revoked": n })).into_response();
    add_cookies(&mut resp, &cleared_cookies());
    Ok(resp)
}

/// A fresh CSRF token, as JSON and as the CSRF cookie (Plane: `GET /auth/get-csrf-token/`).
async fn csrf(Extension(app): Extension<App>) -> ApiResult<Response<Body>> {
    let token = thrust_core::new_csrf_token(&app)?;
    let mut resp = Json(CsrfResponse { csrf_token: token.clone() }).into_response();
    add_cookies(&mut resp, &[set_cookie(CSRF_COOKIE, &token, app.cfg.session_ttl_secs)]);
    Ok(resp)
}

async fn session_info(AuthedVia(a): AuthedVia) -> ApiResult<Json<SessionInfo>> {
    let credential = match a.via {
        thrust_core::Via::Session(_) => CredentialKind::Session,
    };
    Ok(Json(SessionInfo { user_id: a.principal.user_id, credential }))
}

async fn sweep(_: Admin, Extension(app): Extension<App>) -> ApiResult<Json<thrust_core::SweepReport>> {
    Ok(Json(thrust_core::sweep_outbox(&app, 100).await?))
}

/// Test support: put an arbitrary envelope on the events topic (what a redelivery or a second
/// relay would do). It goes through the REAL `Queue` port, so on the Worker it is a real Queues
/// message consumed by the real consumer.
async fn inject_event(_: Admin, Extension(app): Extension<App>, Json(env): Json<Envelope>) -> ApiResult<StatusCode> {
    app.ports
        .queue
        .send(thrust_core::EVENTS_TOPIC, &env, thrust_ports::SendOpts::default())
        .await
        .map_err(CoreError::from)?;
    Ok(StatusCode::ACCEPTED)
}

async fn outbox_depth(_: Admin, Extension(app): Extension<App>) -> ApiResult<Json<serde_json::Value>> {
    let n = thrust_core::outbox_depth(&app).await?;
    Ok(Json(serde_json::json!({ "undispatched": n })))
}

async fn create_work_item(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(WorkspaceSlug, ProjectId)>,
    Json(req): Json<CreateWorkItemRequest>,
) -> ApiResult<(StatusCode, Json<WorkItem>)> {
    let item = thrust_core::create_work_item(&app, &who, &slug, project, req).await?;
    Ok((StatusCode::CREATED, Json(item)))
}

#[derive(Deserialize)]
struct ListQuery {
    scope: Option<String>,
}

async fn list_work_items(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(WorkspaceSlug, ProjectId)>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<WorkItem>>> {
    let vis = match q.scope.as_deref() {
        None | Some("visible") => Visibility::Visible,
        Some("with_deleted") => Visibility::WithDeleted,
        Some(other) => {
            return Err(ApiError(CoreError::Invalid(format!("unknown scope '{other}'"))));
        }
    };
    Ok(Json(thrust_core::list_work_items(&app, &who, &slug, project, vis).await?))
}

async fn delete_work_item(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((slug, project, id)): Path<(WorkspaceSlug, ProjectId, WorkItemId)>,
) -> ApiResult<StatusCode> {
    thrust_core::delete_work_item(&app, &who, &slug, project, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_activity_for_item(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((slug, project, id)): Path<(WorkspaceSlug, ProjectId, WorkItemId)>,
) -> ApiResult<Json<Vec<Activity>>> {
    Ok(Json(thrust_core::list_activity(&app, &who, &slug, project, Some(id)).await?))
}

async fn list_activity_for_project(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(WorkspaceSlug, ProjectId)>,
) -> ApiResult<Json<Vec<Activity>>> {
    Ok(Json(thrust_core::list_activity(&app, &who, &slug, project, None).await?))
}

async fn put_attachment(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((slug, project, id, name)): Path<(WorkspaceSlug, ProjectId, WorkItemId, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let ct = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
    thrust_core::put_attachment(&app, &who, &slug, project, id, &name, body.to_vec(), ct).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_attachment(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((slug, project, id, name)): Path<(WorkspaceSlug, ProjectId, WorkItemId, String)>,
) -> ApiResult<Response<Body>> {
    let blob = thrust_core::get_attachment(&app, &who, &slug, project, id, &name).await?;
    let mut resp = Response::new(Body::from(blob.bytes));
    if let Some(ct) = blob.content_type.and_then(|c| HeaderValue::from_str(&c).ok()) {
        resp.headers_mut().insert(CONTENT_TYPE, ct);
    }
    Ok(resp)
}

async fn delete_attachment(
    Authed(who): Authed,
    Extension(app): Extension<App>,
    Path((slug, project, id, name)): Path<(WorkspaceSlug, ProjectId, WorkItemId, String)>,
) -> ApiResult<StatusCode> {
    thrust_core::delete_attachment(&app, &who, &slug, project, id, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Where the slice's own tenant routes are mounted. It is deliberately not `/api/v1`: that prefix is reserved for
/// routes that are wire-compatible with Plane (ADR-0031, issue #47, FR-06), and no Plane route starts with an
/// underscore. It stays under `/api`, so every edge pattern that covers `/api/*` covers it. The contract suite
/// mirrors this constant (`thrust_contract_tests::SLICE_PREFIX`); the two must be equal or every request 404s.
/// These routes are deleted when the `/api/v1` routes of #58 replace them.
pub const SLICE_PREFIX: &str = "/api/_slice";

/// An axum route pattern under `{SLICE_PREFIX}/auth`.
fn auth_path(tail: &str) -> String {
    format!("{SLICE_PREFIX}/auth{tail}")
}

/// An axum route pattern under `{SLICE_PREFIX}/workspaces/{slug}/projects/{project_id}`.
fn slice_path(tail: &str) -> String {
    format!("{SLICE_PREFIX}/workspaces/{{slug}}/projects/{{project_id}}{tail}")
}

pub fn router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/_info", get(info))
        .route("/_test/panic", get(panic_route))
        .route("/internal/seed", post(seed))
        .route("/internal/sweep", post(sweep))
        .route("/internal/revoke-session", post(revoke_session))
        .route("/internal/set-user-active", post(set_user_active))
        .route(&auth_path("/sign-in/email"), post(sign_in))
        .route(&auth_path("/sign-out"), post(sign_out))
        .route(&auth_path("/sign-out-all"), post(sign_out_all))
        .route(&auth_path("/csrf"), get(csrf))
        .route(&auth_path("/session"), get(session_info))
        .route("/internal/outbox-depth", get(outbox_depth))
        .route("/internal/inject-event", post(inject_event))
        .route(&slice_path("/work-items"), post(create_work_item).get(list_work_items))
        .route(&slice_path("/work-items/{id}"), delete(delete_work_item))
        .route(&slice_path("/work-items/{id}/activity"), get(list_activity_for_item))
        .route(&slice_path("/activity"), get(list_activity_for_project))
        .route(
            &slice_path("/work-items/{id}/attachments/{name}"),
            put(put_attachment).get(get_attachment).delete(delete_attachment),
        )
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
}
