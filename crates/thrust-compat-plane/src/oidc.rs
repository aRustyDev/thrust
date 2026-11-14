//! `/auth/oidc/` and `/auth/oidc/callback/` (X10, D-040): the two routes the stock app's "Sign in with SSO" button needs.
//!
//! The pin's fork adds four GET routes for its OIDC provider (`oidc/`, `oidc/callback/` and the two under `spaces/`, PQ-213); the
//! app's button (`hooks/oauth/core.tsx`) sends the browser to `/auth/oidc/`, drawn only when `/api/instances/` says
//! `is_oidc_enabled`. These two sit over `thrust_core::oidc_start` and `oidc_callback`, which the slice's own routes
//! (`/api/_slice/auth/oidc/...`) already used: the same state, nonce and PKCE verifier, the same refusals, the same session.
//!
//! The login is a chain of top-level navigations, so there is no JSON body for the app to read, unlike the sign-in form's posts
//! (D-022): every failure is a 302 to the app's root with `?error_code=&error_message=`, as the pin's fork answers them.
//!
//! **Dark by default.** A deployment that configures no provider (`Ports::oidc` is `None`, which is every deployment today: the
//! Worker cannot reach a WARP-only issuer until Q-0050.9) answers `/api/instances/` with `is_oidc_enabled: false`, so no button
//! is drawn, and these routes answer `OIDC_NOT_CONFIGURED` to anyone who goes there by hand. The one place the two are tied together is
//! `instance.rs`; a test holds it and a mutant breaks it (`mutant_oidc_flag_always_on`).

use axum::extract::Query;
use axum::http::header::{CACHE_CONTROL, LOCATION};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde::Deserialize;
use thrust_core::{App, CoreError};

use crate::error::Code;
use crate::PlaneError;

/// A failure of the OIDC login as a browser can read it: a 302 to the app's root with the pin's `error_code` and
/// `error_message` (PQ-213), which the root page's `authErrorHandler` reads.
fn failure(code: Code) -> Response {
    #[cfg(not(feature = "mutant_oidc_failure_redirect_no_code"))]
    let location = format!("/?error_code={}&error_message={}", code.number(), code.message());
    // MUTANT (control run only): the redirect has no `error_code`, the one thing the app reads, so a refused login looks like a
    // reload of the sign-in page. The contract suite's refusal tests must catch it.
    #[cfg(feature = "mutant_oidc_failure_redirect_no_code")]
    let location = format!("/?error_message={}", code.message());
    // The location is made of a number and a constant name, all printable ASCII, so this cannot fail; if it ever did, the browser goes to the
    // root with no code and no cookie, which is a refused login that says nothing. (This crate writes no 500 of its own: scripts/check-plane-500.sh.)
    let location = HeaderValue::from_str(&location).unwrap_or_else(|_| HeaderValue::from_static("/"));
    let mut resp = StatusCode::FOUND.into_response();
    resp.headers_mut().insert(LOCATION, location);
    resp.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// What the app is told when the core refuses a step of the login. `None` is a fault of this server and not of the login (the
/// database, the clock): that is a 5xx, as everywhere else, and not a redirect that blames the person.
fn code_for(e: &CoreError) -> Option<Code> {
    match e {
        // no provider is configured: the pin's answer for a provider it has no settings for
        CoreError::NotFound => Some(Code::OidcNotConfigured),
        // the provider vouched for an identity that Thrust will not let in: no verified email, no account for it (Thrust makes none at
        // a login, Q-0050.1), or a deactivated one. The stock app has text for `SIGNUP_DISABLED` ("contact your administrator") and
        // none for the two OIDC codes, so this is the refusal the person is told in words.
        CoreError::Forbidden => Some(Code::SignupDisabled),
        // the provider's answer does not authenticate, the provider cannot be reached, or the state or the code is missing, unknown,
        // used or expired
        CoreError::Unauthenticated | CoreError::Upstream(_) | CoreError::Invalid(_) => {
            Some(Code::OidcOauthProviderError)
        }
        _ => None,
    }
}

/// X10. `GET /auth/oidc/`: the button sends the browser here (`?next_path=` is read by nobody: the login does not carry it, D-040), and
/// this sends it on to the provider's authorization endpoint with a 302.
pub(crate) async fn start(Extension(app): Extension<App>) -> Result<Response, PlaneError> {
    match thrust_core::oidc_start(&app).await {
        Ok(url) => {
            let Ok(location) = HeaderValue::from_str(&url) else {
                return Ok(failure(Code::OidcOauthProviderError));
            };
            let mut resp = StatusCode::FOUND.into_response();
            resp.headers_mut().insert(LOCATION, location);
            resp.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            Ok(resp)
        }
        Err(e) => match code_for(&e) {
            Some(code) => Ok(failure(code)),
            None => Err(e.into()),
        },
    }
}

/// What the provider adds to the redirect it sends the browser back with: a `code` and the `state` it was given, or an `error` when the
/// person said no or the provider failed. Anything else it adds (`iss`, `error_description`, `session_state`) is ignored.
#[derive(Deserialize)]
pub(crate) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// X10. `GET /auth/oidc/callback/`: the provider sends the browser back here. A login the core accepts is the same session as a password
/// sign-in (the same two cookies, `no-store`) and a 302 to the app's root, which decides where the person goes from the profile (the pin
/// redirects the same way, to the bare origin for a new user). Every refusal is `failure`, with no cookie.
pub(crate) async fn callback(
    Extension(app): Extension<App>,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, PlaneError> {
    if app.ports.oidc.is_none() {
        // a deployment with no provider is dark: whatever the browser brings, it is told that none is configured
        return Ok(failure(Code::OidcNotConfigured));
    }
    if q.error.is_some() {
        // the person refused, or the provider failed: no code to exchange, no session, and the stored state expires by itself
        return Ok(failure(Code::OidcOauthProviderError));
    }
    match thrust_core::oidc_callback(&app, q.code.as_deref().unwrap_or(""), q.state.as_deref().unwrap_or("")).await {
        Ok(s) => {
            let mut resp = StatusCode::FOUND.into_response();
            resp.headers_mut().insert(LOCATION, HeaderValue::from_static("/"));
            thrust_api::add_cookies(
                &mut resp,
                &thrust_api::session_cookies(&s.token, &s.csrf_token, app.cfg.session_ttl_secs),
            );
            Ok(resp)
        }
        Err(e) => match code_for(&e) {
            Some(code) => Ok(failure(code)),
            None => Err(e.into()),
        },
    }
}
