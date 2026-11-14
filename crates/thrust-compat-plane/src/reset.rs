//! `/auth/forgot-password/` and `/auth/reset-password/<uidb64>/<token>/` (X10 and X11, D-041): the two routes behind the stock app's "Forgot
//! your password?" link. Everything here was measured on the pinned Plane (2026-11-14; `tools/compat-record`, steps X10 and X11).
//!
//! The first is a JSON route (the app posts JSON through axios; the pin also takes a form): a request is answered `200` with the message
//! the app shows, or `400` with the pin's error body. The second is a native form post of the reset page, so it is answered the way the
//! pin answers it: every outcome is a `302` to the app (the sign-in page with `?success=True`, or the reset page with `?error_code=` and
//! `?error_message=`), nobody is signed in, and a JSON body counts as no password at all, because the pin reads only `request.POST`.
//!
//! **Dark by default** like the OIDC routes (D-040): a deployment with no mail service, or no configured base URL for the link, answers
//! `SMTP_NOT_CONFIGURED` to every request, whatever it asks, as the pin does without `EMAIL_HOST`, and `/api/instances/` says
//! `is_smtp_configured: false`, so the app draws the "ask your admin" line and not the link. The reset route works whatever the
//! configuration is: it needs no mail, only a token.

use axum::body::Bytes;
use axum::extract::Path;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};
use thrust_core::{App, CoreError, PasswordResetError, ResetRequestError};

use crate::error::{AuthError, Code};
use crate::PlaneError;

/// What the app shows after a request that was accepted (the pin's text).
const CHECK_YOUR_EMAIL: &str = "Check your email to reset your password";

/// The last value of `name` in an urlencoded body (Django's `QueryDict.get` answers the last of several), or `None`.
fn form_value(body: &[u8], name: &str) -> Option<String> {
    let pairs: Vec<(String, String)> = serde_urlencoded::from_bytes(body).ok()?;
    pairs.into_iter().rev().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// The address of a forgot-password body: JSON `{"email": ...}` as the app posts it, or a form as the pin also takes. Anything that is
/// not text (a missing member, `null`, a number) is no address, which the pin answers `INVALID_EMAIL` and so does the core.
fn requested_email(body: &[u8]) -> String {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(members)) => members.get("email").and_then(Value::as_str).unwrap_or_default().to_string(),
        Ok(_) => String::new(),
        Err(_) => form_value(body, "email").unwrap_or_default(),
    }
}

/// X10. `POST /auth/forgot-password/`: ask for a link. The pin's order, kept by the core: this deployment can mail (`5025`), the address is
/// one (`5005`), an account has it (`5060`), and then a link is mailed (`200`). A request that mailed nothing because the account's last link
/// is still young, or because the account is deactivated, is answered as one that mailed, so the answer does not say which.
pub(crate) async fn forgot_password(Extension(app): Extension<App>, body: Bytes) -> Result<Response, PlaneError> {
    match thrust_core::request_password_reset(&app, &requested_email(&body)).await {
        Ok(_) => Ok(Json(json!({ "message": CHECK_YOUR_EMAIL })).into_response()),
        Err(ResetRequestError::NotConfigured) => Ok(AuthError::bad_request(Code::SmtpNotConfigured).into_response()),
        Err(ResetRequestError::InvalidEmail) => Ok(AuthError::bad_request(Code::InvalidEmail).into_response()),
        Err(ResetRequestError::UnknownUser) => Ok(AuthError::bad_request(Code::UserDoesNotExist).into_response()),
        // the mail service did not take the message: a fault of the deployment, answered as compat answers every such fault (the pin's generic 500,
        // through `plane_500`), and not an answer the pin has a shape for (its mail is queued, so it never knows)
        Err(ResetRequestError::Mail(m)) => Err(CoreError::Upstream(m).into()),
        Err(ResetRequestError::Core(e)) => Err(e.into()),
    }
}

/// A 302 to the app, which is how every answer of the reset form is given.
fn redirect(location: &str) -> Response {
    let mut resp = StatusCode::FOUND.into_response();
    if let Ok(v) = HeaderValue::from_str(location) {
        resp.headers_mut().insert(LOCATION, v);
    }
    resp.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // an empty body, as the pin's `HttpResponseRedirect` answers it (Django's default type): it does nothing in a browser, and it is the recording's
    resp.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    resp
}

/// A refused reset: back to the reset page with the pin's `error_code` and `error_message` (which the page's `authErrorHandler` reads).
fn refused(code: Code) -> Response {
    #[cfg(not(feature = "mutant_reset_redirect_without_code"))]
    let location = format!("/accounts/reset-password?error_code={}&error_message={}", code.number(), code.message());
    // MUTANT (control run only): the redirect has no `error_code`, the one thing the page reads, so a refused reset looks like a reload.
    #[cfg(feature = "mutant_reset_redirect_without_code")]
    let location = format!("/accounts/reset-password?error_message={}", code.message());
    redirect(&location)
}

/// X11. `POST /auth/reset-password/<uidb64>/<token>/`: the reset page's form. The link is checked, then the password (`password`, from the
/// urlencoded body only), in the pin's order; a success sends the person to the sign-in page, signed in as nobody.
pub(crate) async fn reset_password(
    Extension(app): Extension<App>,
    Path((uidb64, token)): Path<(String, String)>,
    body: Bytes,
) -> Result<Response, PlaneError> {
    let password = form_value(&body, "password").unwrap_or_default();
    match thrust_core::reset_password(&app, &uidb64, &token, &password).await {
        Ok(()) => Ok(redirect("/sign-in?success=True")),
        Err(PasswordResetError::InvalidToken) => Ok(refused(Code::InvalidPasswordToken)),
        Err(PasswordResetError::PasswordMissing) => Ok(refused(Code::InvalidPassword)),
        Err(PasswordResetError::PasswordTooWeak) => Ok(refused(Code::PasswordTooWeak)),
        Err(PasswordResetError::Core(e)) => Err(e.into()),
    }
}
