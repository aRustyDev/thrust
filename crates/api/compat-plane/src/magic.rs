//! `/auth/magic-generate/` and `/auth/magic-sign-in/` (X12 and X13, D-042): the two routes behind the stock app's "Sign in with unique code".
//! Everything here was measured on the pinned Plane (2026-11-16; `tools/compat-record`, steps X12 and X13) except what its own text says was
//! read from the source.
//!
//! The first is a JSON route (the app posts JSON through axios; the pin also takes a form): `200 {"key": "magic_<address>"}` for a request that
//! was accepted, or `400` with the pin's error body. The second is the native form post of the unique-code step (`unique-code.tsx` is the
//! pin's, unchanged: a `<form method=POST>` with the hidden `csrfmiddlewaretoken`, the address, the code and `next_path`), so it is answered
//! the way the pin answers it: every outcome is a `302` to the app, with the session cookies on a success and `?error_code=` on a refusal.
//!
//! **Sign-in only** (the owner, 2026-11-16): `magic-sign-up`, the pin's route that makes an account out of a code, is not built, so a code is
//! made and mailed only for an address an account has (`thrust_core::request_login_code`).
//!
//! **Dark with the reset by email**: a deployment that cannot mail answers `SMTP_NOT_CONFIGURED` to both, as the pin does without
//! `EMAIL_HOST`, and says `is_smtp_configured: false`, so the app draws no "Sign in with unique code" button.

use axum::body::Bytes;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::json;
use thrust_core::{App, CodeRequestError, CodeSignInError, CoreError};

use crate::auth::{quote_keeping_slashes, redirect_path, safe_next_path};
use crate::error::{AuthError, Code};
use crate::reset::{form_value, requested_email};
use crate::PlaneError;

/// X12. `POST /auth/magic-generate/`: ask for a code. The pin's order, kept by the core: the address is one, this deployment can mail
/// (`5025`), and then a code is made and mailed (`200 {"key": "magic_<address>"}`; `5100` once a window has had its four). An address no
/// account has is answered as a mailed one, and nothing is sent: the pin mails it, for its sign-up, which Thrust has not.
///
/// The pin answers an address that is not one with a 500 (its `validate_email` raises, and nothing catches it), which a route of Thrust does
/// not write itself: it is `INVALID_EMAIL`, as email-check says.
pub(crate) async fn generate(Extension(app): Extension<App>, body: Bytes) -> Result<Response, PlaneError> {
    let raw = requested_email(&body);
    let email = thrust_core::normalize_email(&raw);
    match thrust_core::request_login_code(&app, &raw).await {
        Ok(_) => Ok(Json(json!({ "key": format!("magic_{email}") })).into_response()),
        Err(CodeRequestError::NotConfigured) => {
            Ok(AuthError::bad_request(Code::SmtpNotConfigured).with_email(email).into_response())
        }
        Err(CodeRequestError::InvalidEmail) => Ok(AuthError::bad_request(Code::InvalidEmail).into_response()),
        Err(CodeRequestError::Capped) => {
            Ok(AuthError::bad_request(Code::EmailCodeAttemptExhaustedSignIn).with_email(email).into_response())
        }
        // the mail service did not take the message: a fault of the deployment, answered as compat answers every such fault (the pin's
        // generic 500, through `plane_500`), and not an answer the pin has a shape for (its mail is queued, so it never knows)
        Err(CodeRequestError::Mail(m)) => Err(CoreError::Upstream(m).into()),
        Err(CodeRequestError::Core(e)) => Err(e.into()),
    }
}

/// A 302 to the app, which is how every answer of the form is given. The `Location` is the app's own path (the pin's is the same path on
/// an absolute URL: D-041 (7)), so a Host header a caller controls is never written into it.
fn redirect(location: &str) -> Response {
    let mut resp = StatusCode::FOUND.into_response();
    if let Ok(v) = HeaderValue::from_str(location) {
        resp.headers_mut().insert(LOCATION, v);
    }
    resp.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // an empty body, as the pin's `HttpResponseRedirect` answers it (Django's default type)
    resp.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    resp
}

/// A refused sign-in: back to the app's root with the pin's `error_code` and `error_message`, and the address where the pin puts one
/// (`email`), which the page's `authErrorHandler` reads. A `next_path` that is safe goes first, as the pin's `get_safe_redirect_url` puts it,
/// so the person who types the code again still ends up where they were going.
fn refused(code: Code, email: Option<&str>, next_path: &Option<String>) -> Response {
    let mut query = String::new();
    if let Some(p) = safe_next_path(next_path) {
        query.push_str(&format!("next_path={}&", quote_keeping_slashes(&p)));
    }
    let mut pairs: Vec<(&str, String)> = Vec::new();
    // MUTANT (control run only): the redirect has no `error_code`, the one thing the page reads, so a refused code looks like a reload.
    #[cfg(not(feature = "mutant_magic_redirect_without_code"))]
    pairs.push(("error_code", code.number().to_string()));
    pairs.push(("error_message", code.message().to_string()));
    if let Some(e) = email {
        pairs.push(("email", e.to_string()));
    }
    // `urlencode` cannot fail for text pairs; the empty string it would give is a redirect to the root, which is no worse than a failed one
    query.push_str(&serde_urlencoded::to_string(&pairs).unwrap_or_default());
    redirect(&format!("/?{query}"))
}

/// X13. `POST /auth/magic-sign-in/`: the unique-code step's form. The CSRF token of the form is checked first (the pin does, with its
/// failure page: a form that anyone may post to would let any page sign a visitor in as someone else, with a code that page was mailed),
/// then the pin's order: the code and the address are there (`5085`), an account has the address (`5060`), this deployment can mail
/// (`5025`), the code (`5090` wrong, `5100` the last wrong one, `5095` none that works), the account is active (`5019`). A person is
/// signed in by a success, and sent to where they were going or to the app's root.
pub(crate) async fn sign_in(
    Extension(app): Extension<App>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, PlaneError> {
    // MUTANT (control run only): the form's CSRF token is not looked at, so a page that is not ours can sign a visitor in. The CSRF tests must
    // catch it.
    #[cfg(not(feature = "mutant_magic_csrf_unchecked"))]
    {
        let field = form_value(&body, "csrfmiddlewaretoken");
        let cookie = thrust_api::cookie_value(&headers, thrust_api::CSRF_COOKIE);
        if !thrust_core::csrf_ok(cookie, field.as_deref()) {
            return Err(CoreError::Csrf.into());
        }
    }
    #[cfg(feature = "mutant_magic_csrf_unchecked")]
    let _ = &headers;
    let code = form_value(&body, "code").unwrap_or_default().trim().to_string();
    let email = thrust_core::normalize_email(&form_value(&body, "email").unwrap_or_default());
    let next_path = form_value(&body, "next_path");
    if code.is_empty() || email.is_empty() {
        return Ok(refused(Code::MagicSignInEmailCodeRequired, None, &next_path));
    }
    match thrust_core::sign_in_with_code(&app, &email, &code).await {
        Ok(s) => {
            let mut resp = redirect(&redirect_path(&next_path));
            thrust_api::add_cookies(
                &mut resp,
                &thrust_api::session_cookies(&s.token, &s.csrf_token, app.cfg.session_ttl_secs),
            );
            Ok(resp)
        }
        Err(CodeSignInError::UnknownUser) => Ok(refused(Code::UserDoesNotExist, None, &next_path)),
        // the pin's own quirk, kept: its provider names the key (`magic_<address>`) where the other refusals name the address
        Err(CodeSignInError::NotConfigured) => {
            Ok(refused(Code::SmtpNotConfigured, Some(&format!("magic_{email}")), &next_path))
        }
        Err(CodeSignInError::Invalid) => Ok(refused(Code::InvalidMagicCodeSignIn, Some(&email), &next_path)),
        Err(CodeSignInError::Expired) => Ok(refused(Code::ExpiredMagicCodeSignIn, Some(&email), &next_path)),
        Err(CodeSignInError::Exhausted) => Ok(refused(Code::EmailCodeAttemptExhaustedSignIn, Some(&email), &next_path)),
        Err(CodeSignInError::Deactivated) => Ok(refused(Code::UserAccountDeactivated, Some(&email), &next_path)),
        Err(CodeSignInError::RateLimited { .. }) => Ok(refused(Code::RateLimitExceeded, None, &next_path)),
        Err(CodeSignInError::Core(e)) => Err(e.into()),
    }
}
