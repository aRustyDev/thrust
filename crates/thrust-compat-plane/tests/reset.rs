//! `POST /auth/forgot-password/` (X10) and `POST /auth/reset-password/<uidb64>/<token>/` (X11), D-041: the routes behind the stock app's
//! "Forgot your password?" link. The tables below are what the pinned Plane answered (measured 2026-11-14, `tools/compat-record` steps X10
//! and X11); the recording is held by the `parity_` tests, and these state what a recording cannot: every combination of what a body and a
//! link can be, the configurations that make a deployment dark, and what a reset leaves behind.
//!
//! What must hold:
//! * a deployment with no mail service or no base URL answers `SMTP_NOT_CONFIGURED` (5025) to every forgot-password request, whatever it
//!   asks (as the pin does without `EMAIL_HOST`), and says `is_smtp_configured: false`; with both it says `true`;
//! * a configured request is answered in the pin's order (5005, 5060, then the message), takes a JSON or a form body, mails exactly once,
//!   and a second request while the first link is young mails nothing and is answered the same;
//! * a reset is a 302 whatever it comes to, to the sign-in page with `success=True` or to the reset page with the pin's code; the link is
//!   checked before the password (missing, then too weak); only the urlencoded `password` counts (a JSON body is no password, the last of
//!   several wins); and a success ends the account's sessions, refuses the old password, accepts the new one, and sets no cookie.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request};
use axum::{Extension, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use common::*;
use serde_json::{json, Value};
use thrust_adapters_memory::{EmailMode, MemoryEmail};
use thrust_core::{issue_password_reset, sign_up, uidb64, App};
use thrust_ports::EmailSender;
use tower::ServiceExt;

const TARGET: &str = "target@example.test";
const OLD: &str = "correct horse battery staple";
const NEW: &str = "a different strong passphrase";
const SESSION: &str = "__Host-thrust_session";

struct World {
    router: Router,
    app: App,
    mail: Arc<MemoryEmail>,
    db: Arc<thrust_adapters_memory::MemoryDatabase>,
}

struct Answer {
    status: u16,
    location: Option<String>,
    cookies: Vec<String>,
    body: Value,
}

/// The routes over a memory world; `sender` and `base` are what makes a deployment able to mail. The account `TARGET` holds `OLD`.
fn world(mode: EmailMode, sender: bool, base: bool) -> World {
    let mail = Arc::new(MemoryEmail::with_mode(mode));
    let email: Option<Arc<dyn EmailSender>> = sender.then(|| mail.clone() as Arc<dyn EmailSender>);
    let (app, db) = bare_world_parts(None, email, |c| {
        c.public_base_url = base.then(|| "https://projects.example.test".to_string());
    });
    futures_executor::block_on(sign_up(&app, TARGET, OLD)).unwrap();
    let router = thrust_compat_plane::routes().layer(Extension(app.clone()));
    World { router, app, mail, db }
}

fn mailing() -> World {
    world(EmailMode::Accept, true, true)
}

impl World {
    fn send(&self, method: &str, uri: &str, content_type: Option<&str>, body: &[u8], session: Option<&str>) -> Answer {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(ct) = content_type {
            req = req.header(header::CONTENT_TYPE, ct);
        }
        if let Some(token) = session {
            req = req.header(header::COOKIE, format!("{SESSION}={token}"));
        }
        let resp =
            futures_executor::block_on(self.router.clone().oneshot(req.body(Body::from(body.to_vec())).unwrap()))
                .unwrap();
        let (parts, body) = resp.into_parts();
        let bytes = futures_executor::block_on(axum::body::to_bytes(body, usize::MAX)).unwrap();
        Answer {
            status: parts.status.as_u16(),
            location: parts.headers.get(header::LOCATION).map(|v| v.to_str().unwrap().to_string()),
            cookies: parts
                .headers
                .get_all(header::SET_COOKIE)
                .iter()
                .map(|v| v.to_str().unwrap().to_string())
                .collect(),
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }

    fn forgot_json(&self, body: &Value) -> Answer {
        self.send("POST", "/auth/forgot-password/", Some("application/json"), body.to_string().as_bytes(), None)
    }

    fn forgot_raw(&self, content_type: &str, body: &str) -> Answer {
        self.send("POST", "/auth/forgot-password/", Some(content_type), body.as_bytes(), None)
    }

    fn reset(&self, uid: &str, token: &str, content_type: &str, body: &[u8]) -> Answer {
        self.send("POST", &format!("/auth/reset-password/{uid}/{token}/"), Some(content_type), body, None)
    }

    fn reset_form(&self, uid: &str, token: &str, form: &str) -> Answer {
        self.reset(uid, token, "application/x-www-form-urlencoded", form.as_bytes())
    }

    fn target(&self) -> thrust_contracts::UserId {
        user_id(&self.db, TARGET)
    }

    fn issue(&self) -> (String, String) {
        let issued =
            futures_executor::block_on(issue_password_reset(&self.app, self.target())).unwrap().expect("a link");
        (issued.uidb64, issued.token)
    }

    /// The status of signing in with `password`, and of a read with the session the sign-up made.
    fn sign_in_status(&self, password: &str) -> u16 {
        let body = json!({"email": TARGET, "password": password}).to_string();
        self.send("POST", "/auth/sign-in/", Some("application/json"), body.as_bytes(), None).status
    }
}

/// A status, and the pin's `error_code` with its `error_message` when the body has one.
type Coded = (u16, Option<(u64, String)>);

/// What a forgot-password body is called, its content type, the body, and the answer the pin gave.
type Case = (&'static str, &'static str, String, Coded);

fn code(a: &Answer) -> Coded {
    let pair =
        a.body["error_code"].as_u64().map(|c| (c, a.body["error_message"].as_str().unwrap_or_default().to_string()));
    (a.status, pair)
}

fn refused(code: u16, name: &str) -> String {
    format!("/accounts/reset-password?error_code={code}&error_message={name}")
}

// --- forgot-password ---------------------------------------------------------------------------------------------------------------

/// A deployment with no mail service, or no base URL, is dark: every request is 5025 (the pin's answer without `EMAIL_HOST`, whatever the
/// request is), nothing is mailed, and the app is told mail is off. With both, the app is told it is on.
#[test]
fn a_deployment_that_cannot_mail_says_so_to_every_request_and_to_the_app() {
    for (sender, base) in [(false, false), (true, false), (false, true), (true, true)] {
        let w = world(EmailMode::Accept, sender, base);
        let configured = sender && base;
        let instance = w.send("GET", "/api/instances/", None, b"", None);
        assert_eq!(instance.body["config"]["is_smtp_configured"], configured, "sender {sender}, base {base}");
        if configured {
            continue;
        }
        let dark = (400, Some((5025, "SMTP_NOT_CONFIGURED".to_string())));
        for (what, a) in [
            ("a known address", w.forgot_json(&json!({"email": TARGET}))),
            ("an unknown address", w.forgot_json(&json!({"email": "nobody@example.test"}))),
            ("a bad address", w.forgot_json(&json!({"email": "nope"}))),
            ("no address", w.forgot_json(&json!({}))),
            ("null", w.forgot_json(&json!({"email": null}))),
            ("a form", w.forgot_raw("application/x-www-form-urlencoded", "email=target%40example.test")),
            ("no body", w.forgot_raw("application/json", "")),
        ] {
            assert_eq!(code(&a), dark, "sender {sender}, base {base}: {what}");
            assert!(a.cookies.is_empty(), "{what}");
        }
        assert!(w.mail.sent().is_empty(), "sender {sender}, base {base}: nothing is mailed");
    }
}

/// Every body by every address, against what the pin answered once SMTP was configured: 5005 for anything that is not an address (a missing
/// member, null, a number, a list, text that is no body), 5060 for one nobody has, and the message for one somebody has, in JSON or in a form.
#[test]
fn every_forgot_password_body_comes_to_the_answer_the_pin_gave() {
    let message = (200, None);
    let invalid = (400, Some((5005, "INVALID_EMAIL".to_string())));
    let unknown = (400, Some((5060, "USER_DOES_NOT_EXIST".to_string())));
    let cases: Vec<Case> = vec![
        ("a known address", "application/json", json!({"email": TARGET}).to_string(), message.clone()),
        (
            "a known address, padded and in capitals",
            "application/json",
            json!({"email": "  TARGET@EXAMPLE.TEST "}).to_string(),
            message.clone(),
        ),
        (
            "a known address in a form",
            "application/x-www-form-urlencoded",
            "email=target%40example.test".into(),
            message.clone(),
        ),
        (
            "an unknown address",
            "application/json",
            json!({"email": "nobody@example.test"}).to_string(),
            unknown.clone(),
        ),
        (
            "an unknown address in a form",
            "application/x-www-form-urlencoded",
            "email=nobody%40example.test".into(),
            unknown,
        ),
        ("a bad address", "application/json", json!({"email": "nope"}).to_string(), invalid.clone()),
        ("an empty address", "application/json", json!({"email": ""}).to_string(), invalid.clone()),
        ("no address", "application/json", json!({}).to_string(), invalid.clone()),
        ("null", "application/json", json!({"email": null}).to_string(), invalid.clone()),
        ("a number", "application/json", json!({"email": 12345}).to_string(), invalid.clone()),
        ("a list", "application/json", json!(["target@example.test"]).to_string(), invalid.clone()),
        ("text that is no body", "text/plain", "target@example.test".into(), invalid.clone()),
        ("no body", "application/json", String::new(), invalid),
    ];
    let mut messages = 0;
    for (what, content_type, body, want) in cases {
        let w = mailing();
        let a = w.forgot_raw(content_type, &body);
        let got = (
            a.status,
            a.body["error_code"]
                .as_u64()
                .map(|c| (c, a.body["error_message"].as_str().unwrap_or_default().to_string())),
        );
        assert_eq!(got, want, "{what}");
        if want.0 == 200 {
            assert_eq!(a.body, json!({"message": "Check your email to reset your password"}), "{what}");
            assert_eq!(w.mail.sent().len(), 1, "{what}: one mail");
            assert_eq!(w.mail.sent()[0].to, TARGET, "{what}: to the normalised address");
            messages += 1;
        } else {
            assert!(w.mail.sent().is_empty(), "{what}: nothing is mailed");
            assert!(a.body.get("email").is_none(), "{what}: the pin's body has no `email`: {}", a.body);
        }
    }
    assert_eq!(messages, 3, "control: three of the bodies are accepted");
}

/// A second request while the first link is young is answered as the first was and mails nothing; a mail service that fails is the pin's
/// generic 500 (what compat answers for a fault of the deployment, through `plane_500`) and leaves nothing that blocks the next request.
#[test]
fn a_second_request_mails_nothing_and_a_failing_mail_service_is_the_pins_500() {
    let w = mailing();
    assert_eq!(w.forgot_json(&json!({"email": TARGET})).status, 200);
    let again = w.forgot_json(&json!({"email": TARGET}));
    assert_eq!((again.status, again.body), (200, json!({"message": "Check your email to reset your password"})));
    assert_eq!(w.mail.sent().len(), 1, "the second request mailed nothing");

    for mode in [EmailMode::Reject, EmailMode::Unavailable] {
        let w = world(mode, true, true);
        let first = w.forgot_json(&json!({"email": TARGET}));
        assert_eq!(first.status, 500, "{mode:?}");
        assert_eq!(
            w.forgot_json(&json!({"email": TARGET})).status,
            500,
            "{mode:?}: the next request is not held back by a link that never went"
        );
    }
}

// --- reset-password ----------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Link {
    Good,
    GarbageUid,
    UidOfNobody,
    NonUtf8Uid,
    WrongToken,
}

#[derive(Debug, Clone, Copy)]
enum Posted {
    Strong,
    StrongAmongOthers,
    Empty,
    Absent,
    Weak,
    Json,
}

/// Every link by every body: 30 worlds against the table the pin gave. A bad link is 5125 whatever the body says (the link is checked
/// first); a good link meets the body's checks: no password (empty, absent, or sent as JSON) is 5020, a weak one 5021, and a password,
/// the last of several as Django reads it, is the success. What a refusal leaves is what it found; what a success leaves is checked.
#[test]
fn every_reset_comes_to_the_redirect_the_pin_gave() {
    let links = [Link::Good, Link::GarbageUid, Link::UidOfNobody, Link::NonUtf8Uid, Link::WrongToken];
    let bodies = [Posted::Strong, Posted::StrongAmongOthers, Posted::Empty, Posted::Absent, Posted::Weak, Posted::Json];
    let strong = urlencode(NEW);
    let (mut cases, mut successes) = (0, 0);
    for link in links {
        for body in bodies {
            let case = format!("{link:?} x {body:?}");
            let w = mailing();
            let session = futures_executor::block_on(thrust_core::sign_in(&w.app, TARGET, OLD)).unwrap().token;
            let (uid, token) = w.issue();
            let (u, t) = match link {
                Link::Good => (uid.clone(), token.clone()),
                Link::GarbageUid => ("zzz".to_string(), token.clone()),
                Link::UidOfNobody => {
                    (uidb64(thrust_contracts::UserId(thrust_contracts::Uuid::from_u128(0xDEAD))), token.clone())
                }
                Link::NonUtf8Uid => (URL_SAFE_NO_PAD.encode([0xff, 0xfe]), token.clone()),
                Link::WrongToken => (uid.clone(), "abc-def".to_string()),
            };
            let a = match body {
                Posted::Strong => w.reset_form(&u, &t, &format!("password={strong}")),
                Posted::StrongAmongOthers => w.reset_form(
                    &u,
                    &t,
                    &format!("email=x&password=password123&password={strong}&confirm_password={strong}"),
                ),
                Posted::Empty => w.reset_form(&u, &t, "password="),
                Posted::Absent => w.reset_form(&u, &t, "email=x"),
                Posted::Weak => w.reset_form(&u, &t, "password=password123"),
                Posted::Json => w.reset(&u, &t, "application/json", json!({"password": NEW}).to_string().as_bytes()),
            };
            let want = match (link, body) {
                (Link::Good, Posted::Strong | Posted::StrongAmongOthers) => "/sign-in?success=True".to_string(),
                (Link::Good, Posted::Empty | Posted::Absent | Posted::Json) => refused(5020, "INVALID_PASSWORD"),
                (Link::Good, Posted::Weak) => refused(5021, "PASSWORD_TOO_WEAK"),
                _ => refused(5125, "INVALID_PASSWORD_TOKEN"),
            };
            assert_eq!(a.status, 302, "{case}");
            assert_eq!(a.location.as_deref(), Some(want.as_str()), "{case}");
            assert!(a.cookies.is_empty(), "{case}: a reset signs nobody in");
            let live = w.send("GET", "/api/users/me/", None, b"", Some(&session)).status;
            if want.starts_with("/sign-in") {
                successes += 1;
                assert_eq!(live, 401, "{case}: the account's session ended");
                assert_eq!(
                    (w.sign_in_status(OLD), w.sign_in_status(NEW)),
                    (400, 200),
                    "{case}: the old password is refused and the new one accepted"
                );
                assert_eq!(
                    w.reset_form(&uid, &token, &format!("password={strong}")).location.as_deref(),
                    Some(refused(5125, "INVALID_PASSWORD_TOKEN").as_str()),
                    "{case}: the link is used up"
                );
            } else {
                assert_eq!(live, 200, "{case}: a refusal ends no session");
                assert_eq!(
                    (w.sign_in_status(OLD), w.sign_in_status(NEW)),
                    (200, 400),
                    "{case}: the password is as it was"
                );
                // a refusal does not use the mailed link up
                assert_eq!(
                    w.reset_form(&uid, &token, &format!("password={strong}")).location.as_deref(),
                    Some("/sign-in?success=True"),
                    "{case}: the mailed link still works"
                );
            }
            cases += 1;
        }
    }
    // control: a table that reached nothing would pass
    assert_eq!((cases, successes), (30, 2));
}

/// Percent-encode everything but the unreserved characters, as a browser encodes a form field.
fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
