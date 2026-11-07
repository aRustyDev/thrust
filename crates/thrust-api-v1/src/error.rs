//! The errors of Plane's v1 API, spelt as the pin spells them.
//!
//! Two body shapes coexist on the pin and a client reads both: `{"detail": ...}` (what DRF itself answers: a missing or
//! refused credential, a permission, a wrong method) and `{"error": ...}` (what Plane's own handlers answer). A field that
//! fails validation is `{field: [message]}`. Every body here is recorded in the oracle's corpus (`tools/oracle-v1/corpus`);
//! the tests that pin them are in `tests/`.

use axum::body::Body;
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Map, Value};
use thrust_core::CoreError;

/// What the pin answers when a handler raises: its error middleware turns every unhandled exception into this 500.
/// Several requests reach it on purpose (a `per_page` of 0 or -1, a `deleted_at` in a work item's body: EX-001, EX-002), and the
/// owner decided to reproduce them (Q-0033.2), as a handled response and never as a crash of the process (ADR-0031, ADR-0006).
pub const PLANE_500_MESSAGE: &str = "Something went wrong please try again later";

/// A v1 answer that is not a success.
#[derive(Debug)]
pub enum V1Error {
    /// 401, DRF's: no `X-Api-Key` header (or an empty one). Only a **missing** key is a 401.
    NotAuthenticated,
    /// 403, the key middleware's: the key is unknown, expired, revoked, or its user is inactive. The pin does not say which.
    InvalidKey,
    /// 403, DRF's permission class: the caller is not (or not enough of) a member of the workspace or the project.
    PermissionDenied,
    /// 404, Plane's handler: the object is not there, or not one the caller may see.
    NotFound,
    /// 404, the URLconf's: no route matches (an unknown path, or an id that is not a uuid). Answered before any credential is read.
    PageNotFound,
    /// A status with a DRF `{"detail": message}` body.
    Detail(StatusCode, String),
    /// A status with a body the caller built, usually Plane's `{"error": message}` (with an `id` for a duplicate).
    Error(StatusCode, Value),
    /// 400, DRF's field errors: `{field: [message]}`.
    Fields(Map<String, Value>),
    /// 429 with `Retry-After`: the key's allowance for the window is spent.
    RateLimited { retry_after_secs: u64 },
    /// 500, [`PLANE_500_MESSAGE`]. The cause, if there is one, is logged by the composition root and never sent.
    Plane500(Option<String>),
}

/// The pin's 500, for a request it fails on purpose.
pub fn plane_500() -> V1Error {
    V1Error::Plane500(None)
}

impl V1Error {
    /// A 500 for something that should not have happened here: the cause goes to the log, the client gets the pin's body.
    pub fn internal(cause: impl Into<String>) -> Self {
        V1Error::Plane500(Some(cause.into()))
    }

    /// Plane's `{"error": message}` with `status`.
    pub fn error(status: StatusCode, message: &str) -> Self {
        V1Error::Error(status, json!({ "error": message }))
    }
}

fn detail(status: StatusCode, message: &str) -> Response<Body> {
    (status, Json(json!({ "detail": message }))).into_response()
}

fn error(status: StatusCode, message: &str) -> Response<Body> {
    (status, Json(json!({ "error": message }))).into_response()
}

impl IntoResponse for V1Error {
    fn into_response(self) -> Response<Body> {
        match self {
            V1Error::NotAuthenticated => {
                detail(StatusCode::UNAUTHORIZED, "Authentication credentials were not provided.")
            }
            V1Error::InvalidKey => detail(StatusCode::FORBIDDEN, "Given API token is not valid"),
            V1Error::PermissionDenied => {
                detail(StatusCode::FORBIDDEN, "You do not have permission to perform this action.")
            }
            V1Error::NotFound => error(StatusCode::NOT_FOUND, "The requested resource does not exist."),
            V1Error::PageNotFound => error(StatusCode::NOT_FOUND, "Page not found."),
            V1Error::Detail(status, message) => detail(status, &message),
            V1Error::Error(status, body) => (status, Json(body)).into_response(),
            V1Error::Fields(fields) => (StatusCode::BAD_REQUEST, Json(Value::Object(fields))).into_response(),
            V1Error::RateLimited { retry_after_secs } => {
                let body = json!({ "error_code": 5900, "error_message": "RATE_LIMIT_EXCEEDED" });
                let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
                resp.headers_mut().insert(RETRY_AFTER, HeaderValue::from(retry_after_secs));
                resp
            }
            V1Error::Plane500(cause) => {
                let mut resp = error(StatusCode::INTERNAL_SERVER_ERROR, PLANE_500_MESSAGE);
                if let Some(cause) = cause {
                    resp.extensions_mut().insert(thrust_api::InternalErrorDetail(cause));
                }
                resp
            }
        }
    }
}

impl From<CoreError> for V1Error {
    /// The mapping for an error nothing route-specific has claimed. A route that the pin answers differently (a project that is
    /// not there is a 403 for the entity routes and a 404 for the project's own) maps the two cases itself before it gets here.
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Unauthenticated => V1Error::NotAuthenticated,
            CoreError::InvalidApiKey => V1Error::InvalidKey,
            CoreError::Forbidden | CoreError::Csrf => V1Error::PermissionDenied,
            CoreError::NotFound => V1Error::NotFound,
            CoreError::RateLimited { retry_after_secs } => V1Error::RateLimited { retry_after_secs },
            CoreError::Invalid(message) => V1Error::Detail(StatusCode::BAD_REQUEST, message),
            CoreError::Conflict(message) => V1Error::error(StatusCode::CONFLICT, &message),
            CoreError::TooLarge => V1Error::Detail(StatusCode::PAYLOAD_TOO_LARGE, "Request entity too large.".into()),
            CoreError::Upstream(cause) | CoreError::Internal(cause) => V1Error::Plane500(Some(cause)),
        }
    }
}

impl From<thrust_ports::DbError> for V1Error {
    fn from(e: thrust_ports::DbError) -> Self {
        CoreError::from(e).into()
    }
}
