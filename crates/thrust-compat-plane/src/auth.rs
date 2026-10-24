//! `/auth/...`: the routes the sign-up and sign-in pages call (S03 to S06).
//!
//! The pinned Plane answers its sign-up and sign-in form posts with a 302 whose `Location` carries `?error_code=` on a
//! failure. Thrust's copy of the React auth client posts JSON to the same paths instead (issue #44, Q-0009.1, D-022):
//! a success is a 200 with `{"redirect_path": ...}` and the session cookies, a failure is a 400 (a 429 for the rate
//! limit) with `{"error_code": ..., "error_message": ..., "email": ...}`, the codes and messages of Plane's
//! `AUTHENTICATION_ERROR_CODES`, so the client's `authErrorHandler` is unchanged. `email-check` and `get-csrf-token`
//! are JSON in Plane too and keep their shapes.
//!
//! Plane's order of checks is kept, because it decides which code a request with two faults gets.

use axum::body::Bytes;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;
use thrust_core::{App, CoreError};

use crate::error::{AuthError, Code};
use crate::PlaneError;

/// What a sign-up, sign-in or email-check body may carry. A body that is not a JSON object of strings is an empty one,
/// as a form post with no fields is to Plane: the answer is then the "required" error, not a framework 4xx.
#[derive(Deserialize, Default)]
struct AuthBody {
    email: Option<String>,
    password: Option<String>,
    next_path: Option<String>,
}

fn parse(body: &Bytes) -> AuthBody {
    serde_json::from_slice(body).unwrap_or_default()
}

/// Plane reads a missing field as Python's `False` and formats it with `str()`, so an absent email is "False" in the
/// payload of a "required" error and an empty one is "".
fn payload_email(raw: &Option<String>) -> String {
    raw.clone().unwrap_or_else(|| "False".to_string())
}

fn present(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| !s.is_empty())
}

/// S03. A fresh CSRF token as JSON and as the CSRF cookie. The pinned Plane's cookie is `csrftoken`; Thrust's is the
/// `__Host-` one the shared router sets (`thrust_api::CSRF_COOKIE`), so the stock app, which reads the token from the
/// body, is not affected.
pub(crate) async fn get_csrf_token(Extension(app): Extension<App>) -> Result<Response, PlaneError> {
    let token = thrust_core::new_csrf_token(&app)?;
    let mut resp = Json(json!({ "csrf_token": token })).into_response();
    thrust_api::add_cookies(
        &mut resp,
        &[thrust_api::set_cookie(thrust_api::CSRF_COOKIE, &token, app.cfg.session_ttl_secs)],
    );
    Ok(resp)
}

/// S04. Whether an account exists for the email, and which login the page should offer. Thrust has no SMTP and no magic
/// code, so it is always the password ("CREDENTIAL"), which is what the pin answers when SMTP is not configured.
pub(crate) async fn email_check(Extension(app): Extension<App>, body: Bytes) -> Result<Response, PlaneError> {
    let b = parse(&body);
    if !present(&b.email) {
        return Ok(AuthError::bad_request(Code::EmailRequired).into_response());
    }
    let email = thrust_core::normalize_email(b.email.as_deref().unwrap_or_default());
    if !thrust_core::is_valid_email(&email) {
        return Ok(AuthError::bad_request(Code::InvalidEmail).into_response());
    }
    let existing = thrust_core::email_exists(&app, &email).await?;
    Ok(Json(json!({ "existing": existing, "status": "CREDENTIAL" })).into_response())
}

/// What Plane's `_contains_suspicious_patterns` looks for in a `next_path` (compared lower case).
const SUSPICIOUS: &[&str] = &[
    "javascript:", "data:", "vbscript:", "file:", "ftp:", "%2e%2e", "%2f%2f", "%5c%5c", "<script", "<iframe", "<object",
    "<embed", "<form", "onload=", "onerror=", "onclick=",
];

/// A relative path the browser may be sent to: one slash first and no second, no backslash or control character, no
/// `..`, none of Plane's suspicious patterns, at most 500 characters. Plane's `validate_next_path` repairs what it can
/// (it keeps the path of an absolute URL); this one refuses anything it would have to repair, which is never less safe.
fn safe_next_path(raw: &Option<String>) -> Option<String> {
    let p = raw.as_deref()?;
    let lower = p.to_lowercase();
    let ok = p.len() <= 500
        && p.starts_with('/')
        && !p.starts_with("//")
        && !p.contains("..")
        && !p.chars().any(|c| c == '\\' || c.is_control())
        && !SUSPICIOUS.iter().any(|s| lower.contains(s));
    ok.then(|| p.to_string())
}

/// Percent-encode everything but the unreserved characters and `/` (Python's `quote(s, safe="/")`).
fn quote_keeping_slashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Where the browser goes after a sign-in or sign-up: the app's root, with `?next_path=` when the client asked for a
/// place and the place is safe. The root page decides the rest (onboarding, a workspace, the workspace list) from the
/// profile, which is why Plane's own redirect for a new user is the bare origin. Plane writes `next_path` into the
/// query unencoded, so a `&` or `#` in it corrupts the query; this encodes them, which differs only for such paths.
fn redirect_path(next_path: &Option<String>) -> String {
    match safe_next_path(next_path) {
        Some(p) => format!("/?next_path={}", quote_keeping_slashes(&p)),
        None => "/".to_string(),
    }
}

fn signed_in(app: &App, s: &thrust_core::SignedIn, next_path: &Option<String>) -> Response {
    let mut resp = Json(json!({ "redirect_path": redirect_path(next_path) })).into_response();
    thrust_api::add_cookies(&mut resp, &thrust_api::session_cookies(&s.token, &s.csrf_token, app.cfg.session_ttl_secs));
    resp
}

/// S05. Make an account and sign it in. Plane's order: both fields present, a valid email, no account with it, a strong
/// password.
pub(crate) async fn sign_up(Extension(app): Extension<App>, body: Bytes) -> Result<Response, PlaneError> {
    let b = parse(&body);
    if !present(&b.email) || !present(&b.password) {
        return Ok(AuthError::bad_request(Code::RequiredEmailPasswordSignUp).with_email(payload_email(&b.email)).into_response());
    }
    let email = thrust_core::normalize_email(b.email.as_deref().unwrap_or_default());
    let password = b.password.as_deref().unwrap_or_default();
    if !thrust_core::is_valid_email(&email) {
        return Ok(AuthError::bad_request(Code::InvalidEmailSignUp).with_email(email).into_response());
    }
    if thrust_core::email_exists(&app, &email).await? {
        return Ok(AuthError::bad_request(Code::UserAlreadyExist).with_email(email).into_response());
    }
    if !thrust_core::password_is_strong(password) {
        return Ok(AuthError::bad_request(Code::PasswordTooWeak).with_email(email).into_response());
    }
    match thrust_core::sign_up(&app, &email, password).await {
        Ok(s) => Ok(signed_in(&app, &s, &b.next_path)),
        // two sign-ups racing for one email: the unique index decided, and this one lost
        Err(CoreError::Conflict(_)) => Ok(AuthError::bad_request(Code::UserAlreadyExist).with_email(email).into_response()),
        Err(e) => Err(e.into()),
    }
}

/// S06. Sign in with an email and a password. Plane's order: both fields present, a valid email, an account with it
/// (else "no such user"), the right password.
pub(crate) async fn sign_in(Extension(app): Extension<App>, body: Bytes) -> Result<Response, PlaneError> {
    let b = parse(&body);
    if !present(&b.email) || !present(&b.password) {
        return Ok(AuthError::bad_request(Code::RequiredEmailPasswordSignIn).with_email(payload_email(&b.email)).into_response());
    }
    let email = thrust_core::normalize_email(b.email.as_deref().unwrap_or_default());
    let password = b.password.as_deref().unwrap_or_default();
    if !thrust_core::is_valid_email(&email) {
        return Ok(AuthError::bad_request(Code::InvalidEmailSignIn).with_email(email).into_response());
    }
    if !thrust_core::email_exists(&app, &email).await? {
        return Ok(AuthError::bad_request(Code::UserDoesNotExist).with_email(email).into_response());
    }
    match thrust_core::sign_in(&app, &email, password).await {
        Ok(s) => Ok(signed_in(&app, &s, &b.next_path)),
        // a wrong password, an account with no password, a deactivated one, or an over-long field: one answer for all
        Err(CoreError::Unauthenticated | CoreError::Invalid(_)) => {
            Ok(AuthError::bad_request(Code::AuthenticationFailedSignIn).with_email(email).into_response())
        }
        Err(CoreError::RateLimited { retry_after_secs }) => {
            Ok(AuthError::rate_limited(retry_after_secs).with_email(email).into_response())
        }
        Err(e) => Err(e.into()),
    }
}
