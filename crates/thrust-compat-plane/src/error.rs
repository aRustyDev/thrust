//! Plane's error bodies. A body is added here only when a fixture recorded from the pin shows its shape; until then a
//! failure keeps the shape `thrust-api` gives it.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;
use thrust_core::CoreError;

/// The codes of Plane's `AUTHENTICATION_ERROR_CODES` that the routes here can answer (the pin's
/// `apps/api/plane/authentication/adapter/error.py`), with the message Plane gives each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    InvalidEmail,
    EmailRequired,
    PasswordTooWeak,
    UserAlreadyExist,
    RequiredEmailPasswordSignUp,
    InvalidEmailSignUp,
    UserDoesNotExist,
    AuthenticationFailedSignIn,
    RequiredEmailPasswordSignIn,
    InvalidEmailSignIn,
    RateLimitExceeded,
}

impl Code {
    pub fn number(self) -> u16 {
        match self {
            Code::InvalidEmail => 5005,
            Code::EmailRequired => 5010,
            Code::PasswordTooWeak => 5021,
            Code::UserAlreadyExist => 5030,
            Code::RequiredEmailPasswordSignUp => 5040,
            Code::InvalidEmailSignUp => 5045,
            Code::UserDoesNotExist => 5060,
            Code::AuthenticationFailedSignIn => 5065,
            Code::RequiredEmailPasswordSignIn => 5070,
            Code::InvalidEmailSignIn => 5075,
            Code::RateLimitExceeded => 5900,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Code::InvalidEmail => "INVALID_EMAIL",
            Code::EmailRequired => "EMAIL_REQUIRED",
            Code::PasswordTooWeak => "PASSWORD_TOO_WEAK",
            Code::UserAlreadyExist => "USER_ALREADY_EXIST",
            Code::RequiredEmailPasswordSignUp => "REQUIRED_EMAIL_PASSWORD_SIGN_UP",
            Code::InvalidEmailSignUp => "INVALID_EMAIL_SIGN_UP",
            Code::UserDoesNotExist => "USER_DOES_NOT_EXIST",
            Code::AuthenticationFailedSignIn => "AUTHENTICATION_FAILED_SIGN_IN",
            Code::RequiredEmailPasswordSignIn => "REQUIRED_EMAIL_PASSWORD_SIGN_IN",
            Code::InvalidEmailSignIn => "INVALID_EMAIL_SIGN_IN",
            Code::RateLimitExceeded => "RATE_LIMIT_EXCEEDED",
        }
    }
}

/// An authentication failure as the auth client reads it: the body of Plane's `get_error_dict()`. A 400, except the rate
/// limit (a 429 with `Retry-After`), which Plane also answers with the error dict.
pub struct AuthError {
    status: StatusCode,
    code: Code,
    email: Option<String>,
    retry_after_secs: Option<u64>,
}

impl AuthError {
    pub fn bad_request(code: Code) -> Self {
        Self { status: StatusCode::BAD_REQUEST, code, email: None, retry_after_secs: None }
    }

    pub fn rate_limited(retry_after_secs: u64) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: Code::RateLimitExceeded,
            email: None,
            retry_after_secs: Some(retry_after_secs),
        }
    }

    /// The `email` of the error's payload (Plane puts it there for the sign-in and sign-up errors, not for email-check's).
    pub fn with_email(mut self, email: String) -> Self {
        self.email = Some(email);
        self
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response<Body> {
        let mut body = json!({"error_code": self.code.number(), "error_message": self.code.message()});
        if let Some(email) = self.email {
            body["email"] = json!(email);
        }
        let mut resp = (self.status, Json(body)).into_response();
        if let Some(s) = self.retry_after_secs {
            if let Ok(v) = axum::http::HeaderValue::from_str(&s.to_string()) {
                resp.headers_mut().insert(axum::http::header::RETRY_AFTER, v);
            }
        }
        resp
    }
}

pub enum PlaneError {
    /// No session, or an expired one. The React client sends every 401 to the sign-in page, so this status means only
    /// that (research/0003 section 9, row 2); a role failure is a 403.
    Unauthenticated,
    /// A 400 with a body the caller built: DRF's `{field: [message]}` for a field that is wrong, or one of the pin's
    /// single-message bodies.
    BadRequest(serde_json::Value),
    /// DRF's field errors, `{field: [message]}`, as a 400.
    Fields(serde_json::Map<String, serde_json::Value>),
    /// A 403 with the body the route's own permission class gives (they differ per route on the pin).
    Forbidden(serde_json::Value),
    /// A 409 with a body the caller built (workspace creation's lost race for a slug).
    Conflict(serde_json::Value),
    /// Anything whose Plane shape has not been recorded yet.
    Other(CoreError),
}

impl From<CoreError> for PlaneError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Unauthenticated => PlaneError::Unauthenticated,
            other => PlaneError::Other(other),
        }
    }
}

impl From<thrust_api::ApiError> for PlaneError {
    fn from(e: thrust_api::ApiError) -> Self {
        e.0.into()
    }
}

impl IntoResponse for PlaneError {
    fn into_response(self) -> Response<Body> {
        match self {
            // The body DRF gives (fixture S02).
            PlaneError::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                Json(json!({"detail": "Authentication credentials were not provided."})),
            )
                .into_response(),
            PlaneError::BadRequest(body) => (StatusCode::BAD_REQUEST, Json(body)).into_response(),
            PlaneError::Fields(errors) => (StatusCode::BAD_REQUEST, Json(serde_json::Value::Object(errors))).into_response(),
            PlaneError::Forbidden(body) => (StatusCode::FORBIDDEN, Json(body)).into_response(),
            PlaneError::Conflict(body) => (StatusCode::CONFLICT, Json(body)).into_response(),
            PlaneError::Other(e) => thrust_api::ApiError(e).into_response(),
        }
    }
}
