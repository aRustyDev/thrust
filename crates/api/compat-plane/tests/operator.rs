//! The operator's door, `/internal/operator/*` (Q-F7, issue #46), driven in process as the composition roots mount it: the shared
//! router with the compat routes beside it, an `ApiConfig` as an extension. The core's rules (what a reset does to a password and
//! to sessions) are stated in `thrust-core/tests/operator.rs`; this is the door: when it is there, who it lets in, what it answers,
//! and what it leaves behind.
//!
//! What must hold: with the switch off the routes do not exist (404 for everyone, the token or not); with it on, exactly one
//! request opens them, the one that carries the configured operator token, which must be long enough to be one, in the operator
//! token's own header (the test routes' token opens nothing here, and neither does a prefix of the operator's, nor the operator's
//! with something after it); a request that is refused changes nothing; the routes make an account and reset a password; and no
//! answer, header or audit line holds the password it was sent.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Method, Request};
use axum::{Extension, Router};
use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::json;
use thrust_api::{ApiConfig, OperatorAudit, MIN_OPERATOR_TOKEN_LEN, OPERATOR_TOKEN_HEADER};
use thrust_core::{sign_in, sign_up, App, CoreError};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const ADMIN_TOKEN: &str = "fedcba9876543210fedcba9876543210";
const ACCOUNT: &str = "someone@example.test";
const PASSWORD: &str = "Workspaces-Sample-Pa55word-Zebra-81";
const NEW_PASSWORD: &str = "A-different-Pa55phrase-Quokka-27";

fn config(operator_routes: bool, operator_token: Option<&str>) -> ApiConfig {
    ApiConfig {
        // the test routes are on and have a token of their own, as on a contract target: the operator's door must not read either
        test_routes: true,
        admin_token: Some(ADMIN_TOKEN.into()),
        operator_routes,
        operator_token: operator_token.map(str::to_string),
        target: "test".into(),
        adapters: "memory".into(),
    }
}

/// The shared router and the compat routes over a memory world, as `thrust-server` mounts them, and the world's `App`.
fn mounted(cfg: ApiConfig) -> (Router, App) {
    let (app, _db) = bare_world();
    let router =
        thrust_api::router().merge(thrust_compat_plane::routes()).layer(Extension(app.clone())).layer(Extension(cfg));
    (router, app)
}

/// What one request got back, and the audit line the response carried for the root to log.
struct Sent {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    audit: Option<String>,
}

fn send(router: &Router, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&str>) -> Sent {
    let mut builder = Request::builder().method(method.parse::<Method>().expect("method")).uri(path);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let body = match body {
        Some(b) => {
            builder = builder.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = block_on(router.clone().oneshot(builder.body(body).expect("request"))).expect("infallible");
    let status = resp.status().as_u16();
    let audit = resp.extensions().get::<OperatorAudit>().map(|a| a.0.clone());
    let headers =
        resp.headers().iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())).collect();
    let body = block_on(axum::body::to_bytes(resp.into_body(), usize::MAX)).expect("body").to_vec();
    Sent { status, headers, body, audit }
}

/// A call to one of the two routes with the operator's token (or the header the caller names), and an email and password.
fn operator(router: &Router, route: &str, headers: &[(&str, &str)], email: &str, password: &str) -> Sent {
    let body = json!({"email": email, "password": password}).to_string();
    send(router, "POST", &format!("/internal/operator/{route}"), headers, Some(&body))
}

const CREATE: &str = "users";
const RESET: &str = "users/password";

fn with_token() -> [(&'static str, &'static str); 1] {
    [(OPERATOR_TOKEN_HEADER, TOKEN)]
}

fn session_is_live(router: &Router, token: &str) -> bool {
    send(router, "GET", "/api/users/me/", &[("authorization", &format!("Bearer {token}"))], None).status == 200
}

/// Nothing this exchange sent in the clear is in what came back or what the root would log.
fn assert_no_password_leaves(sent: &Sent, password: &str, what: &str) {
    let text = String::from_utf8_lossy(&sent.body);
    assert!(!text.contains(password), "{what}: the body holds the password: {text}");
    for (name, value) in &sent.headers {
        assert!(!value.contains(password), "{what}: the {name} header holds the password");
    }
    if let Some(line) = &sent.audit {
        assert!(!line.contains(password), "{what}: the audit line holds the password: {line}");
    }
}

// --- the switch ------------------------------------------------------------------------------------------------------

/// With the switch off the routes are not there: 404 for the right token, the wrong one and none, and nothing is made or
/// changed. (A 401 would say there is a door.)
#[rstest]
fn a_door_that_is_off_is_not_there(
    #[values(CREATE, RESET)] route: &str,
    #[values("the operator's token", "a wrong token", "no token")] header: &str,
) {
    let (router, app) = mounted(config(false, Some(TOKEN)));
    let existing = block_on(sign_up(&app, ACCOUNT, PASSWORD)).unwrap();
    let headers: Vec<(&str, &str)> = match header {
        "the operator's token" => with_token().to_vec(),
        "a wrong token" => vec![(OPERATOR_TOKEN_HEADER, "not-the-token")],
        _ => vec![],
    };
    let email = if route == CREATE { "new@example.test" } else { ACCOUNT };
    let sent = operator(&router, route, &headers, email, NEW_PASSWORD);
    assert_eq!(sent.status, 404, "{route} with {header}");
    // control: nothing happened. No account was made, the password is still the old one, the session stands
    assert!(!block_on(thrust_core::email_exists(&app, "new@example.test")).unwrap());
    assert!(block_on(sign_in(&app, ACCOUNT, PASSWORD)).is_ok());
    assert!(session_is_live(&router, &existing.token));
}

// --- who it lets in --------------------------------------------------------------------------------------------------

/// What a request carries, against a door that is on and has the token `TOKEN`.
#[derive(Debug, Clone)]
enum Header {
    Absent,
    /// The operator's token, in its own header.
    Right,
    /// Another token of the same shape.
    Other(String),
    /// The test routes' token, in the operator's header, and again in its own.
    AdminTokenInOperatorsSlot,
    AdminTokenInAdminsSlot,
    /// The operator's token, in the test routes' header.
    OperatorTokenInAdminsSlot,
    /// The operator's token with its last character gone, and with one more after it: a comparison that stops early passes one.
    Truncated,
    Extended,
}

fn headers_for(h: &Header, token: &str) -> Vec<(String, String)> {
    let op = OPERATOR_TOKEN_HEADER.to_string();
    match h {
        Header::Absent => vec![],
        Header::Right => vec![(op, token.to_string())],
        Header::Other(t) => vec![(op, t.clone())],
        Header::AdminTokenInOperatorsSlot => vec![(op, ADMIN_TOKEN.to_string())],
        Header::AdminTokenInAdminsSlot => vec![("x-admin-token".to_string(), ADMIN_TOKEN.to_string())],
        Header::OperatorTokenInAdminsSlot => vec![("x-admin-token".to_string(), token.to_string())],
        Header::Truncated => vec![(op, token[..token.len().saturating_sub(1)].to_string())],
        Header::Extended => vec![(op, format!("{token}x"))],
    }
}

/// Why a request got the answer it did: the generator control needs every one of these reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Why {
    SwitchOff,
    NoTokenConfigured,
    TokenTooShort,
    WrongToken,
    Open,
}

/// The answer the door must give, from what it was configured with and what the request carried. The whole rule, in one place.
fn expected(switch: bool, configured: &Option<String>, h: &Header) -> (u16, Why) {
    if !switch {
        return (404, Why::SwitchOff);
    }
    match configured {
        None => (401, Why::NoTokenConfigured),
        Some(t) if t.len() < MIN_OPERATOR_TOKEN_LEN => (401, Why::TokenTooShort),
        Some(_) if matches!(h, Header::Right) => (204, Why::Open),
        Some(_) => (401, Why::WrongToken),
    }
}

fn configured() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        1 => Just(None),
        2 => "[a-zA-Z0-9]{1,31}".prop_map(Some),
        4 => "[a-zA-Z0-9]{32,48}".prop_map(Some),
    ]
}

fn header() -> impl Strategy<Value = Header> {
    prop_oneof![
        1 => Just(Header::Absent),
        4 => Just(Header::Right),
        2 => "[a-zA-Z0-9]{1,48}".prop_map(Header::Other),
        1 => Just(Header::AdminTokenInOperatorsSlot),
        1 => Just(Header::AdminTokenInAdminsSlot),
        1 => Just(Header::OperatorTokenInAdminsSlot),
        1 => Just(Header::Truncated),
        1 => Just(Header::Extended),
    ]
}

/// One request to the create route against a door built from `switch` and `configured`, and what it answered. The token a request
/// carries is the configured one (`Right` is "the operator's token" whatever it is, and a too-short one is the interesting case).
fn knock(switch: bool, configured: &Option<String>, h: &Header) -> (u16, bool) {
    let (router, app) = mounted(config(switch, configured.as_deref()));
    let carried = configured.clone().unwrap_or_else(|| TOKEN.to_string());
    let headers = headers_for(h, &carried);
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let sent = operator(&router, CREATE, &borrowed, ACCOUNT, PASSWORD);
    (sent.status, block_on(thrust_core::email_exists(&app, ACCOUNT)).unwrap())
}

proptest! {
    #![proptest_config(proptest_config(128))]

    /// The door opens exactly when the switch is on and the request carries the configured token, which is at least
    /// `MIN_OPERATOR_TOKEN_LEN` long: 404 with the switch off whatever else is true, 401 for every other request, 204 for that one.
    /// An account is made if and only if the door opened.
    #[test]
    fn prop_the_door_opens_for_the_configured_long_token_and_nothing_else(
        switch in any::<bool>(),
        configured in configured(),
        h in header(),
    ) {
        let (want, _) = expected(switch, &configured, &h);
        let (got, made) = knock(switch, &configured, &h);
        prop_assert_eq!(got, want, "switch={} configured={:?} request={:?}", switch, configured, h);
        prop_assert_eq!(made, want == 204, "an account exists exactly when the door opened");
    }
}

/// Control for the property: its generator reaches every reason, so no branch of `expected` is untested (a generator that never
/// built a configured token under the minimum would pass a door that takes a short token).
#[test]
fn prop_the_door_generator_reaches_every_reason() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let strategy = (any::<bool>(), configured(), header());
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    let mut short_and_equal = false;
    for _ in 0..400 {
        let (switch, configured, h) = strategy.new_tree(&mut runner).unwrap().current();
        seen.insert(expected(switch, &configured, &h).1);
        short_and_equal |=
            switch && matches!(&configured, Some(t) if t.len() < MIN_OPERATOR_TOKEN_LEN) && matches!(h, Header::Right);
    }
    let all = [Why::SwitchOff, Why::NoTokenConfigured, Why::TokenTooShort, Why::WrongToken, Why::Open];
    let missing: Vec<_> = all.iter().filter(|w| !seen.contains(w)).collect();
    assert!(missing.is_empty(), "never reached: {missing:?}");
    assert!(short_and_equal, "never sent a too-short token that is exactly the configured one");
}

/// The pinned cases the spec names: the test routes' token is not the operator's (the one door that must not open another), and a
/// token of the minimum length is a token.
#[rstest]
#[case::the_test_routes_token_in_the_operators_header(vec![(OPERATOR_TOKEN_HEADER, ADMIN_TOKEN)], 401)]
#[case::the_test_routes_token_in_its_own_header(vec![("x-admin-token", ADMIN_TOKEN)], 401)]
#[case::the_operators_token(vec![(OPERATOR_TOKEN_HEADER, TOKEN)], 204)]
fn the_test_routes_token_opens_nothing_here(#[case] headers: Vec<(&str, &str)>, #[case] status: u16) {
    assert_eq!(TOKEN.len(), MIN_OPERATOR_TOKEN_LEN, "the pinned token is the shortest one a door takes");
    let (router, app) = mounted(config(true, Some(TOKEN)));
    let sent = operator(&router, CREATE, &headers, ACCOUNT, PASSWORD);
    assert_eq!(sent.status, status);
    assert_eq!(block_on(thrust_core::email_exists(&app, ACCOUNT)).unwrap(), status == 204);
}

// --- what it does ----------------------------------------------------------------------------------------------------

/// Make an account, sign in with it, make it again (409), reset its password (204, its sessions end), and the refusals that leave
/// everything as it was: a weak password (400), an email with no account (404), a body that is not the two fields.
#[test]
fn the_routes_make_an_account_and_reset_its_password() {
    let (router, app) = mounted(config(true, Some(TOKEN)));

    let made = operator(&router, CREATE, &with_token(), " Someone@Example.TEST ", PASSWORD);
    assert_eq!(made.status, 204);
    assert!(made.body.is_empty(), "no body: the audit line, not the answer, says which user");
    let audit = made.audit.expect("an audit line");
    assert!(audit.starts_with("operator created a user: user "), "{audit}");
    let first = block_on(sign_in(&app, ACCOUNT, PASSWORD)).expect("the new account signs in");
    assert!(audit.ends_with(&first.user_id.0.to_string()), "{audit}: names the user");
    assert!(session_is_live(&router, &first.token));

    let again = operator(&router, CREATE, &with_token(), ACCOUNT, NEW_PASSWORD);
    assert_eq!(again.status, 409, "the email is taken");

    for (what, route, email, password, status) in [
        ("a weak password", RESET, ACCOUNT, "password123", 400),
        ("an email with no account", RESET, "nobody@example.test", NEW_PASSWORD, 404),
        ("a weak password on create", CREATE, "weak@example.test", "password123", 400),
        ("a malformed email on create", CREATE, "not-an-email", NEW_PASSWORD, 400),
    ] {
        let sent = operator(&router, route, &with_token(), email, password);
        assert_eq!(sent.status, status, "{what}");
        assert!(sent.audit.is_none(), "{what}: a refusal leaves no audit line of something done");
        assert_no_password_leaves(&sent, password, what);
    }
    // control: the refusals changed nothing: the password is the old one and the session stands
    assert!(block_on(sign_in(&app, ACCOUNT, PASSWORD)).is_ok());
    assert!(session_is_live(&router, &first.token));
    assert!(!block_on(thrust_core::email_exists(&app, "weak@example.test")).unwrap());

    let reset = operator(&router, RESET, &with_token(), ACCOUNT, NEW_PASSWORD);
    assert_eq!(reset.status, 204);
    assert!(reset.body.is_empty());
    let audit = reset.audit.expect("an audit line");
    assert!(audit.starts_with("operator set a password: user "), "{audit}");
    assert!(audit.ends_with(&first.user_id.0.to_string()), "{audit}");
    assert!(!session_is_live(&router, &first.token), "the session ends with the old password");
    assert!(matches!(block_on(sign_in(&app, ACCOUNT, PASSWORD)), Err(CoreError::Unauthenticated)));
    let signed_in = block_on(sign_in(&app, ACCOUNT, NEW_PASSWORD)).expect("the new password signs in");
    assert_eq!(signed_in.user_id, first.user_id);
    assert!(session_is_live(&router, &signed_in.token));
}

/// A body that is not the two fields is refused, whatever the route says about the token, and makes nothing.
#[rstest]
fn a_body_that_is_not_an_email_and_a_password_is_refused(
    #[values(CREATE, RESET)] route: &str,
    #[values(
        r#"{}"#,
        r#"{"email":"someone@example.test"}"#,
        r#"{"password":"x"}"#,
        r#"[]"#,
        r#"not json"#,
        r#"{"email":1,"password":2}"#
    )]
    body: &str,
) {
    let (router, app) = mounted(config(true, Some(TOKEN)));
    let sent = send(&router, "POST", &format!("/internal/operator/{route}"), &with_token(), Some(body));
    assert!((400..500).contains(&sent.status), "{route} {body}: {}", sent.status);
    assert_ne!(sent.status, 401, "{route} {body}: the token was right");
    assert!(!block_on(thrust_core::email_exists(&app, ACCOUNT)).unwrap(), "nothing was made");
}

/// Only POST: a GET with the right token is not a way to make anything.
#[rstest]
fn the_routes_take_post_only(#[values("GET", "PUT", "DELETE")] method: &str, #[values(CREATE, RESET)] route: &str) {
    let (router, _app) = mounted(config(true, Some(TOKEN)));
    let sent = send(&router, method, &format!("/internal/operator/{route}"), &with_token(), None);
    assert!(matches!(sent.status, 404 | 405), "{method} {route}: {}", sent.status);
}

/// The password is in nothing that leaves: not the answers to a success or a refusal, not their headers, not the audit line.
#[test]
fn no_answer_header_or_audit_line_holds_the_password() {
    let (router, _app) = mounted(config(true, Some(TOKEN)));
    let secret = "Zq9-Correct-Horse-Pass-7731";
    for (what, route, email, password) in [
        ("create", CREATE, ACCOUNT, secret),
        ("create again", CREATE, ACCOUNT, secret),
        ("reset", RESET, ACCOUNT, secret),
        ("reset of nobody", RESET, "nobody@example.test", secret),
    ] {
        let sent = operator(&router, route, &with_token(), email, password);
        assert_no_password_leaves(&sent, password, what);
    }
    // control: the check can see a password: an echo of the body would be found
    let echo = Sent { status: 200, headers: vec![], body: secret.as_bytes().to_vec(), audit: None };
    assert!(std::panic::catch_unwind(|| assert_no_password_leaves(&echo, secret, "control")).is_err());
}

#[test]
fn a_session_token_is_not_the_operators_token() {
    // a person's session, as a bearer or a cookie, is not a way in
    let (router, app) = mounted(config(true, Some(TOKEN)));
    let s = block_on(sign_up(&app, ACCOUNT, PASSWORD)).unwrap();
    let bearer = format!("Bearer {}", s.token);
    let cookie = format!("__Host-thrust_session={}", s.token);
    for headers in [vec![("authorization", bearer.as_str())], vec![("cookie", cookie.as_str())]] {
        let sent = operator(&router, RESET, &headers, ACCOUNT, NEW_PASSWORD);
        assert_eq!(sent.status, 401, "{headers:?}");
    }
    assert!(block_on(sign_in(&app, ACCOUNT, PASSWORD)).is_ok(), "the password is the old one");
}
