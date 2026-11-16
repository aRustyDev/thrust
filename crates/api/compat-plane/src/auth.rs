//! `/auth/...`: the routes the sign-up and sign-in pages call (S03 to S06), the one the sign-out menu calls (X06), and the one
//! Profile -> Security calls (X09).
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
    "javascript:",
    "data:",
    "vbscript:",
    "file:",
    "ftp:",
    "%2e%2e",
    "%2f%2f",
    "%5c%5c",
    "<script",
    "<iframe",
    "<object",
    "<embed",
    "<form",
    "onload=",
    "onerror=",
    "onclick=",
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

/// X06. Sign out: end the session this request holds and clear both cookies. The pin's view (`SignOutAuthEndpoint`) logs the session
/// out and redirects to the app's origin, and answers the same redirect when anything goes wrong, a request with no session
/// included; so does this, as JSON like the sign-in (D-036): 200 `{"redirect_path": "/"}` for every request that is not refused for
/// its CSRF token. A session that is expired, revoked or unknown has nothing to end, and an API key is no session: a leaked key
/// must not end the user's sessions (as the shared router's `sign_out` says).
pub(crate) async fn sign_out(
    Extension(app): Extension<App>,
    auth: Result<thrust_api::AuthedVia, thrust_api::ApiError>,
) -> Result<Response, PlaneError> {
    match auth {
        Ok(thrust_api::AuthedVia(a)) => {
            if let thrust_core::Via::Session(hash) = a.via {
                thrust_core::sign_out(&app, &hash).await?;
            }
        }
        // a cookie mutation without the CSRF token, where the deployment enforces it (ADR-0052), is refused as every one is
        Err(thrust_api::ApiError(CoreError::Csrf)) => return Err(CoreError::Csrf.into()),
        Err(_) => {}
    }
    let mut resp = Json(json!({ "redirect_path": "/" })).into_response();
    thrust_api::add_cookies(&mut resp, &thrust_api::cleared_cookies());
    Ok(resp)
}

/// What a body field is to Plane's `request.data.get(name, False)` followed by `if not value`: nothing (absent, `null`, `false`, an empty
/// string, list or object, a zero), or text, or something else that is not nothing.
enum Field<'a> {
    Missing,
    Text(&'a str),
    NotText,
}

fn field<'a>(body: &'a serde_json::Value, name: &str) -> Field<'a> {
    use serde_json::Value;
    // `Value::get` is `None` for a body that is not an object (an array, a string, a number or `null`): the same as an empty one
    match body.get(name) {
        None | Some(Value::Null | Value::Bool(false)) => Field::Missing,
        Some(Value::String(s)) if s.is_empty() => Field::Missing,
        Some(Value::String(s)) => Field::Text(s),
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f == 0.0) => Field::Missing,
        Some(Value::Array(a)) if a.is_empty() => Field::Missing,
        Some(Value::Object(o)) if o.is_empty() => Field::Missing,
        Some(_) => Field::NotText,
    }
}

/// X09. Change the signed-in caller's own password (what Profile -> Security calls). Plane's order, which decides the code of a
/// request with two faults: who is asking (401; a session cookie without the CSRF token is refused as every one is), the old
/// password present, the new one present, the old one right, the new one strong. Success is 200 `{"message": ...}` and a new session
/// in the cookies: the caller stays signed in and every other session of the account ends (D-039).
///
/// Only a session may do it: an API key is a script's credential, and a leaked one must not take the account over. The pin's
/// internal API knows only sessions, so a key is nobody there, and a 401 here. A value that is not text is what the pin fails on
/// (a 500 for the new password; a wrong old password for the old one): here a new password that is not text is a weak one, said after
/// the old password has been checked, and a body that is not a JSON object is an empty one.
pub(crate) async fn change_password(
    Extension(app): Extension<App>,
    auth: Result<thrust_api::AuthedVia, thrust_api::ApiError>,
    body: Bytes,
) -> Result<Response, PlaneError> {
    let thrust_api::AuthedVia(caller) = auth?;
    // MUTANT (control run only): an API key may change the password. The contract suite's change-password test must catch it.
    #[cfg(not(feature = "mutant_change_password_allows_api_key"))]
    if !matches!(caller.via, thrust_core::Via::Session(_)) {
        return Err(PlaneError::Unauthenticated);
    }
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    let wrong_old = || AuthError::bad_request(Code::IncorrectOldPassword).with_error("Old password is not correct");
    // Plane's order: both are looked at for being nothing before either is looked at for being right, so an old password that is not
    // text (a wrong one, on the pin) is not heard until the new one has been found to be there
    let old = field(&body, "old_password");
    if matches!(old, Field::Missing) {
        return Ok(AuthError::bad_request(Code::MissingPassword).with_error("Old password is missing").into_response());
    }
    let new = field(&body, "new_password");
    if matches!(new, Field::Missing) {
        return Ok(AuthError::bad_request(Code::MissingPassword)
            .with_error("Old or new password is missing")
            .into_response());
    }
    let Field::Text(old) = old else { return Ok(wrong_old().into_response()) };
    // not text: refused as too weak, which is said after the old password is checked, as the pin would have
    let new = if let Field::Text(new) = new { new } else { "" };
    match thrust_core::change_password(&app, &caller.principal, old, new).await {
        Ok(thrust_core::PasswordChange::Changed(s)) => {
            let mut resp = Json(json!({ "message": "Password updated successfully" })).into_response();
            thrust_api::add_cookies(
                &mut resp,
                &thrust_api::session_cookies(&s.token, &s.csrf_token, app.cfg.session_ttl_secs),
            );
            Ok(resp)
        }
        Ok(thrust_core::PasswordChange::WrongOldPassword) => Ok(wrong_old().into_response()),
        Ok(thrust_core::PasswordChange::TooWeak) => Ok(AuthError::bad_request(Code::PasswordTooWeak).into_response()),
        Err(CoreError::RateLimited { retry_after_secs }) => {
            Ok(AuthError::rate_limited(retry_after_secs).into_response())
        }
        Err(e) => Err(e.into()),
    }
}

/// S05. Make an account and sign it in. Plane's order: both fields present, a valid email, no account with it, a strong
/// password.
pub(crate) async fn sign_up(Extension(app): Extension<App>, body: Bytes) -> Result<Response, PlaneError> {
    let b = parse(&body);
    if !present(&b.email) || !present(&b.password) {
        return Ok(AuthError::bad_request(Code::RequiredEmailPasswordSignUp)
            .with_email(payload_email(&b.email))
            .into_response());
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
        Err(CoreError::Conflict(_)) => {
            Ok(AuthError::bad_request(Code::UserAlreadyExist).with_email(email).into_response())
        }
        Err(e) => Err(e.into()),
    }
}

/// S06. Sign in with an email and a password. Plane's order: both fields present, a valid email, an account with it
/// (else "no such user"), the right password.
pub(crate) async fn sign_in(Extension(app): Extension<App>, body: Bytes) -> Result<Response, PlaneError> {
    let b = parse(&body);
    if !present(&b.email) || !present(&b.password) {
        return Ok(AuthError::bad_request(Code::RequiredEmailPasswordSignIn)
            .with_email(payload_email(&b.email))
            .into_response());
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
