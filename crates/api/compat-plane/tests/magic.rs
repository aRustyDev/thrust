//! `POST /auth/magic-generate/` (X12) and `POST /auth/magic-sign-in/` (X13), D-042: the routes behind the stock app's "Sign in with unique
//! code". The tables below are what the pinned Plane answered (measured 2026-11-16, `tools/compat-record` steps X12 and X13); the recording is
//! held by the `parity_` tests, and these state what a recording cannot: every combination of what a body and a form can be, the
//! configurations that make a deployment dark, and what the form leaves behind.
//!
//! What must hold:
//! * a deployment with no mail service or no base URL answers `SMTP_NOT_CONFIGURED` (5025) to a request for a code, and to a sign-in with
//!   a code, whatever the code is (but not to an address no account has, which is 5060 first, as in the pin), and says
//!   `is_magic_login_enabled` and `is_smtp_configured` true exactly when it can mail;
//! * a request for a code is answered in the pin's order (the address is one, the deployment can mail), mails once for an address an account
//!   has and nothing for one it has not, and answers both with the same body; the fifth request in a window is `5100`;
//! * the form is a 302 whatever it comes to: the pin's order of refusals, each at `/?error_code=..&error_message=..` with the address where
//!   the pin puts it; a success sets both cookies and goes where the person was going or to the root; the form's CSRF token is asked for
//!   first and a form without it changes nothing;
//! * whatever a form carries, the answer is a 302 that has a `Location`, and never a server error.

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
use common::*;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Value};
use thrust_adapters_memory::{EmailMode, MemoryEmail};
use thrust_core::{set_user_active, sign_up, App};
use thrust_ports::EmailSender;
use tower::ServiceExt;

const TARGET: &str = "target@example.test";
const OLD: &str = "correct horse battery staple";
const SESSION: &str = "__Host-thrust_session";
const CSRF: &str = "__Host-thrust_csrf";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";

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

fn world_with(mode: EmailMode, sender: bool, base: bool, rate: u32) -> World {
    let mail = Arc::new(MemoryEmail::with_mode(mode));
    let email: Option<Arc<dyn EmailSender>> = sender.then(|| mail.clone() as Arc<dyn EmailSender>);
    let (app, db) = bare_world_parts(None, email, |c| {
        c.public_base_url = base.then(|| "https://projects.example.test".to_string());
        c.sign_in_rate_limit = rate;
    });
    futures_executor::block_on(sign_up(&app, TARGET, OLD)).unwrap();
    let router = thrust_compat_plane::routes().layer(Extension(app.clone()));
    World { router, app, mail, db }
}

fn world(sender: bool, base: bool) -> World {
    world_with(EmailMode::Accept, sender, base, 100_000)
}

fn mailing() -> World {
    world(true, true)
}

/// How a form carries its CSRF token: the field and the cookie agree, the field is missing, or it is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Csrf {
    Ok,
    NoField,
    WrongField,
    NoCookie,
}

impl World {
    fn send(&self, method: &str, uri: &str, content_type: Option<&str>, body: &[u8], cookie: Option<String>) -> Answer {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(ct) = content_type {
            req = req.header(header::CONTENT_TYPE, ct);
        }
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
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

    fn generate_json(&self, body: &Value) -> Answer {
        self.send("POST", "/auth/magic-generate/", Some("application/json"), body.to_string().as_bytes(), None)
    }

    fn generate_raw(&self, content_type: &str, body: &str) -> Answer {
        self.send("POST", "/auth/magic-generate/", Some(content_type), body.as_bytes(), None)
    }

    /// The unique-code step's form, as the app posts it: urlencoded, with the hidden `csrfmiddlewaretoken`, and the CSRF cookie the page got
    /// from `GET /auth/get-csrf-token/`.
    fn form(&self, fields: &[(&str, &str)], csrf: Csrf) -> Answer {
        let mut pairs: Vec<(&str, &str)> = fields.to_vec();
        match csrf {
            Csrf::Ok | Csrf::NoCookie => pairs.push(("csrfmiddlewaretoken", TOKEN)),
            Csrf::WrongField => pairs.push(("csrfmiddlewaretoken", "not-the-token")),
            Csrf::NoField => {}
        }
        let body = serde_urlencoded::to_string(&pairs).unwrap();
        let cookie = (csrf != Csrf::NoCookie).then(|| format!("{CSRF}={TOKEN}"));
        self.send("POST", "/auth/magic-sign-in/", Some("application/x-www-form-urlencoded"), body.as_bytes(), cookie)
    }

    fn sign_in(&self, email: &str, code: &str) -> Answer {
        self.form(&[("email", email), ("code", code)], Csrf::Ok)
    }

    fn ask(&self, email: &str) -> Answer {
        self.generate_json(&json!({ "email": email }))
    }

    /// The code in the newest message mailed to `email`: the six digits that end its subject.
    fn code_for(&self, email: &str) -> String {
        let sent = self.mail.sent();
        let m = sent.iter().rev().find(|m| m.to == email).expect("a message to the address");
        m.subject.rsplit(' ').next().unwrap().to_string()
    }

    fn user(&self) -> thrust_contracts::UserId {
        user_id(&self.db, TARGET)
    }
}

/// A status, and the pin's `error_code` with its `error_message` when the body has one.
type Coded = (u16, Option<(u64, String)>);

fn code(a: &Answer) -> Coded {
    let pair =
        a.body["error_code"].as_u64().map(|c| (c, a.body["error_message"].as_str().unwrap_or_default().to_string()));
    (a.status, pair)
}

/// The redirect of a refused form: the app's root with the pin's code and message, then the address where the pin puts one.
fn refused(code: u16, name: &str, email: Option<&str>) -> String {
    let mut s = format!("/?error_code={code}&error_message={name}");
    if let Some(e) = email {
        s.push_str(&format!("&email={}", serde_urlencoded::to_string([("", e)]).unwrap().trim_start_matches('=')));
    }
    s
}

// --- magic-generate ---------------------------------------------------------------------------------------------------------------

/// A deployment with no mail service, or no base URL, is dark: a request for a code is 5025 (with the address where the pin puts it),
/// nothing is mailed, and the app is told mail is off. With both, the app is told it is on, for the password reset and for the code alike.
#[test]
fn a_deployment_that_cannot_mail_says_so_to_the_request_to_the_form_and_to_the_app() {
    for (sender, base) in [(false, false), (true, false), (false, true), (true, true)] {
        let w = world(sender, base);
        let configured = sender && base;
        let instance = w.send("GET", "/api/instances/", None, b"", None);
        assert_eq!(instance.body["config"]["is_smtp_configured"], configured, "sender {sender}, base {base}");
        assert_eq!(instance.body["config"]["is_magic_login_enabled"], configured, "sender {sender}, base {base}");
        if configured {
            continue;
        }
        for (what, a) in [("a known address", w.ask(TARGET)), ("an unknown address", w.ask("nobody@example.test"))] {
            assert_eq!(code(&a), (400, Some((5025, "SMTP_NOT_CONFIGURED".to_string()))), "{what}");
            assert!(a.body["email"].is_string(), "{what}: the pin names the address in the body: {}", a.body);
            assert!(a.cookies.is_empty(), "{what}");
        }
        // an address that is not one is the same refusal when mail is on: the check of the address comes first
        assert_eq!(code(&w.ask("nope")), (400, Some((5005, "INVALID_EMAIL".to_string()))));
        // the form: an address nobody has is 5060 first, as in the pin; a known one finds the deployment dark, with the pin's `magic_` quirk
        let unknown = w.sign_in("nobody@example.test", "123456");
        assert_eq!(unknown.location.as_deref(), Some(refused(5060, "USER_DOES_NOT_EXIST", None).as_str()));
        let known = w.sign_in(TARGET, "123456");
        assert_eq!(
            known.location.as_deref(),
            Some(refused(5025, "SMTP_NOT_CONFIGURED", Some("magic_target@example.test")).as_str()),
            "sender {sender}, base {base}"
        );
        assert!(w.mail.sent().is_empty(), "sender {sender}, base {base}: nothing is mailed");
    }
}

/// Every body by every address, against what the pin answered once SMTP and the code were on: `200 {"key": "magic_<address>"}` for any address
/// (the pin mails any, for its sign-up: Thrust mails only one an account has), and for anything that is not an address `INVALID_EMAIL`
/// (the pin's own answer is a 500, which Thrust does not write).
#[test]
fn every_magic_generate_body_comes_to_its_answer() {
    let key = |address: &str| (200, Some(format!("magic_{address}")));
    let invalid: (u16, Option<String>) = (400, None);
    type Case = (&'static str, &'static str, String, (u16, Option<String>), usize);
    let cases: Vec<Case> = vec![
        ("a known address", "application/json", json!({"email": TARGET}).to_string(), key(TARGET), 1),
        (
            "a known address, padded and in capitals",
            "application/json",
            json!({"email": "  TARGET@EXAMPLE.TEST "}).to_string(),
            key(TARGET),
            1,
        ),
        (
            "a known address in a form",
            "application/x-www-form-urlencoded",
            "email=target%40example.test".into(),
            key(TARGET),
            1,
        ),
        (
            "an unknown address: answered as a mailed one, mailed nothing",
            "application/json",
            json!({"email": "nobody@example.test"}).to_string(),
            key("nobody@example.test"),
            0,
        ),
        ("a bad address", "application/json", json!({"email": "nope"}).to_string(), invalid.clone(), 0),
        ("an empty address", "application/json", json!({"email": ""}).to_string(), invalid.clone(), 0),
        ("no address", "application/json", json!({}).to_string(), invalid.clone(), 0),
        ("null", "application/json", json!({"email": null}).to_string(), invalid.clone(), 0),
        ("a number", "application/json", json!({"email": 12345}).to_string(), invalid.clone(), 0),
        ("a list", "application/json", json!(["target@example.test"]).to_string(), invalid.clone(), 0),
        ("text that is no body", "text/plain", "target@example.test".into(), invalid.clone(), 0),
        ("no body", "application/json", String::new(), invalid, 0),
    ];
    for (what, content_type, body, (status, key), mails) in cases {
        let w = mailing();
        let a = w.generate_raw(content_type, &body);
        assert_eq!(a.status, status, "{what}");
        match key {
            Some(k) => assert_eq!(a.body, json!({ "key": k }), "{what}"),
            None => assert_eq!(code(&a), (400, Some((5005, "INVALID_EMAIL".to_string()))), "{what}"),
        }
        assert_eq!(w.mail.sent().len(), mails, "{what}: messages mailed");
        if mails == 1 {
            assert_eq!(w.mail.sent()[0].to, TARGET, "{what}: to the normalised address");
        }
    }
}

/// A window allows four codes and refuses the fifth with the pin's 5100, naming the address; an address no account has is never counted
/// (nothing is stored for it), so it is answered the same however often it is asked; a mail service that fails is the pin's generic 500 and
/// leaves nothing that counts against the next request.
#[test]
fn the_fifth_request_in_a_window_is_refused_and_a_stranger_is_never_counted() {
    let w = mailing();
    for n in 1..=4 {
        assert_eq!(w.ask(TARGET).status, 200, "request {n}");
    }
    let fifth = w.ask(TARGET);
    assert_eq!(code(&fifth), (400, Some((5100, "EMAIL_CODE_ATTEMPT_EXHAUSTED_SIGN_IN".to_string()))));
    assert_eq!(fifth.body["email"], TARGET);
    assert_eq!(w.mail.sent().len(), 4, "the fifth was mailed nothing");
    for n in 1..=10 {
        assert_eq!(w.ask("nobody@example.test").status, 200, "request {n} for an address nobody has");
    }
    for mode in [EmailMode::Reject, EmailMode::Unavailable] {
        let w = world_with(mode, true, true, 100_000);
        for n in 1..=6 {
            assert_eq!(w.ask(TARGET).status, 500, "{mode:?}, request {n}: never capped, because no code ever counted");
        }
    }
}

// --- magic-sign-in ----------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Email {
    Empty,
    Unknown,
    Known,
}

/// The form's order of refusals, every combination: the CSRF token first (a 403, nothing else happens, and the code is still good after
/// it), then the code and the address being there (5085), an account with the address (5060), then the code, which here was never asked for
/// (5095, with the address). The pin's answers, from the measurement.
#[rstest]
fn the_form_is_refused_in_the_pins_order(
    #[values(Csrf::Ok, Csrf::NoField, Csrf::WrongField, Csrf::NoCookie)] csrf: Csrf,
    #[values(Email::Empty, Email::Unknown, Email::Known)] email: Email,
    #[values("", "123456")] code: &str,
) {
    let w = mailing();
    let address = match email {
        Email::Empty => "",
        Email::Unknown => "nobody@example.test",
        Email::Known => TARGET,
    };
    let a = w.form(&[("email", address), ("code", code)], csrf);
    if csrf != Csrf::Ok {
        assert_eq!(a.status, 403, "no CSRF token, no sign-in");
        assert!(a.cookies.is_empty() && a.location.is_none());
        return;
    }
    let want = if code.is_empty() || address.is_empty() {
        refused(5085, "MAGIC_SIGN_IN_EMAIL_CODE_REQUIRED", None)
    } else if matches!(email, Email::Unknown) {
        refused(5060, "USER_DOES_NOT_EXIST", None)
    } else {
        refused(5095, "EXPIRED_MAGIC_CODE_SIGN_IN", Some(TARGET))
    };
    assert_eq!((a.status, a.location.as_deref()), (302, Some(want.as_str())));
    assert!(a.cookies.is_empty(), "a refusal sets no cookie");
}

/// A form whose CSRF token is not good is refused whole: the code in it is not tried, so a page that is not ours cannot use up a person's
/// tries, and cannot sign them in.
#[rstest]
fn a_form_without_its_csrf_token_changes_nothing(
    #[values(Csrf::NoField, Csrf::WrongField, Csrf::NoCookie)] csrf: Csrf,
) {
    let w = mailing();
    assert_eq!(w.ask(TARGET).status, 200);
    let code = w.code_for(TARGET);
    for _ in 0..8 {
        let a = w.form(&[("email", TARGET), ("code", &code)], csrf);
        assert_eq!((a.status, a.cookies.len()), (403, 0));
    }
    // eight refused forms later the code still has all its tries, and signs the person in
    let ok = w.sign_in(TARGET, &code);
    assert_eq!(ok.location.as_deref(), Some("/"), "{:?}", ok.location);
    assert_eq!(ok.cookies.len(), 2);
}

/// A right code is a 302 to the root with both of Thrust's cookies, and the session works; it does not change the password, and it is used
/// up: the same code a second time is 5095.
#[test]
fn a_right_code_signs_the_person_in_once() {
    let w = mailing();
    assert_eq!(w.ask(TARGET).status, 200);
    let code = w.code_for(TARGET);
    let a = w.sign_in(&TARGET.to_uppercase(), &format!(" {code} "));
    assert_eq!((a.status, a.location.as_deref()), (302, Some("/")));
    let cookie = |name: &str| a.cookies.iter().find(|c| c.starts_with(&format!("{name}="))).cloned();
    let (session, csrf) = (cookie(SESSION).expect("a session cookie"), cookie(CSRF).expect("a CSRF cookie"));
    assert!(session.contains("HttpOnly") && session.contains("Secure"), "{session}");
    assert!(csrf.contains("Secure"), "{csrf}");
    let token = session.split(';').next().unwrap().to_string();
    let me = w.send("GET", "/api/users/me/", None, b"", Some(token));
    assert_eq!((me.status, me.body["email"].as_str()), (200, Some(TARGET)));
    let again = w.sign_in(TARGET, &code);
    assert_eq!(again.location.as_deref(), Some(refused(5095, "EXPIRED_MAGIC_CODE_SIGN_IN", Some(TARGET)).as_str()));
    // the password is as it was
    let by_password = w.send(
        "POST",
        "/auth/sign-in/",
        Some("application/json"),
        json!({"email": TARGET, "password": OLD}).to_string().as_bytes(),
        None,
    );
    assert_eq!(by_password.status, 200);
}

/// Where a form sends the person: a safe `next_path` is kept (encoded as the password sign-in encodes it, D-022), and goes first in a
/// refusal's query; one that is not safe is dropped, from a success and from a refusal alike.
#[rstest]
fn the_next_path_is_kept_when_it_is_safe(
    #[values(
        None,
        Some("/a/b"),
        Some("/a?x=1&y=2"),
        Some("https://evil.example/x"),
        Some("//evil.example"),
        Some("javascript:alert(1)"),
        Some("/a/../b"),
        Some("")
    )]
    next: Option<&str>,
    #[values(true, false)] right: bool,
) {
    let w = mailing();
    assert_eq!(w.ask(TARGET).status, 200);
    let code = if right { w.code_for(TARGET) } else { "000000".to_string() };
    let mut fields = vec![("email", TARGET), ("code", code.as_str())];
    if let Some(n) = next {
        fields.push(("next_path", n));
    }
    let a = w.form(&fields, Csrf::Ok);
    let safe = match next {
        Some("/a/b") => Some("/a/b".to_string()),
        Some("/a?x=1&y=2") => Some("/a%3Fx%3D1%26y%3D2".to_string()),
        _ => None,
    };
    let want = if right {
        safe.map_or("/".to_string(), |p| format!("/?next_path={p}"))
    } else {
        let tail = refused(5090, "INVALID_MAGIC_CODE_SIGN_IN", Some(TARGET));
        safe.map_or(tail.clone(), |p| format!("/?next_path={p}&{}", tail.trim_start_matches("/?")))
    };
    assert_eq!(a.location.as_deref(), Some(want.as_str()), "next_path {next:?}, right {right}");
}

/// The tries: a code allows five, a wrong one is 5090 up to the fifth, which is 5100 and ends the code, after which the code (the right one
/// too) is 5095; and the right one at the fifth try still signs the person in.
#[rstest]
fn a_code_allows_five_tries_and_the_fifth_right_one_still_counts(#[values(false, true)] right_at_the_last: bool) {
    let w = mailing();
    assert_eq!(w.ask(TARGET).status, 200);
    let code = w.code_for(TARGET);
    for n in 1..=4 {
        let a = w.sign_in(TARGET, "000000");
        assert_eq!(
            a.location.as_deref(),
            Some(refused(5090, "INVALID_MAGIC_CODE_SIGN_IN", Some(TARGET)).as_str()),
            "try {n}"
        );
    }
    if right_at_the_last {
        let a = w.sign_in(TARGET, &code);
        assert_eq!((a.location.as_deref(), a.cookies.len()), (Some("/"), 2));
        return;
    }
    let fifth = w.sign_in(TARGET, "000000");
    assert_eq!(
        fifth.location.as_deref(),
        Some(refused(5100, "EMAIL_CODE_ATTEMPT_EXHAUSTED_SIGN_IN", Some(TARGET)).as_str())
    );
    let after = w.sign_in(TARGET, &code);
    assert_eq!(after.location.as_deref(), Some(refused(5095, "EXPIRED_MAGIC_CODE_SIGN_IN", Some(TARGET)).as_str()));
    assert!(after.cookies.is_empty());
}

/// A code that is right for an account that has been deactivated is used up and refused (the pin's 5019); no session is made.
#[test]
fn a_right_code_of_a_deactivated_account_is_refused_and_used_up() {
    let w = mailing();
    assert_eq!(w.ask(TARGET).status, 200);
    let code = w.code_for(TARGET);
    assert!(futures_executor::block_on(set_user_active(&w.app, w.user(), false)).unwrap());
    let a = w.sign_in(TARGET, &code);
    assert_eq!(a.location.as_deref(), Some(refused(5019, "USER_ACCOUNT_DEACTIVATED", Some(TARGET)).as_str()));
    assert!(a.cookies.is_empty());
    let again = w.sign_in(TARGET, &code);
    assert_eq!(again.location.as_deref(), Some(refused(5095, "EXPIRED_MAGIC_CODE_SIGN_IN", Some(TARGET)).as_str()));
}

/// The tries of an address are limited like a password's; past the limit the form is the pin's 5900 (`RATE_LIMIT_EXCEEDED`), with no address.
#[rstest]
fn the_tries_of_an_address_are_limited(#[values(1u32, 2, 3)] limit: u32) {
    let w = world_with(EmailMode::Accept, true, true, limit);
    assert_eq!(w.ask(TARGET).status, 200);
    let code = w.code_for(TARGET);
    for _ in 0..limit {
        assert_eq!(w.sign_in(TARGET, "000000").status, 302);
    }
    let over = w.sign_in(TARGET, &code);
    assert_eq!(over.location.as_deref(), Some(refused(5900, "RATE_LIMIT_EXCEEDED", None).as_str()));
    assert!(over.cookies.is_empty(), "the right code, over the limit, signs nobody in");
}

// --- whatever a form carries --------------------------------------------------------------------------------------------------------

proptest! {
    /// Whatever the three fields of the form hold (any text: control characters, quotes, very long values, an address that is not one), with a
    /// good CSRF token, the answer is a 302 that has a `Location` (an invalid header value would drop it), that stays in the app (a path, no
    /// second slash, no line break), and the app has no server error: the route never writes a 500.
    #[test]
    fn prop_the_form_always_answers_with_a_redirect_that_stays_in_the_app(
        email in ".{0,80}", code in ".{0,80}", next in prop::option::of(".{0,120}"),
    ) {
        let w = mailing();
        let mut fields = vec![("email", email.as_str()), ("code", code.as_str())];
        if let Some(n) = next.as_deref() {
            fields.push(("next_path", n));
        }
        let a = w.form(&fields, Csrf::Ok);
        prop_assert_eq!(a.status, 302);
        let loc = a.location.expect("a Location");
        prop_assert!(loc.starts_with("/?") || loc == "/", "{loc}");
        prop_assert!(!loc.starts_with("//") && !loc.contains(['\r', '\n']), "{loc}");
    }

    /// Whatever a request for a code carries, in whatever type, the answer is a 200 or a 400 with the pin's body, never a server error.
    #[test]
    fn prop_a_request_for_a_code_is_never_a_server_error(
        body in prop::collection::vec(any::<u8>(), 0..200),
        content_type in prop_oneof![Just("application/json"), Just("application/x-www-form-urlencoded"), Just("text/plain")],
    ) {
        let w = mailing();
        let a = w.send("POST", "/auth/magic-generate/", Some(content_type), &body, None);
        prop_assert!(a.status == 200 || a.status == 400, "{}", a.status);
    }
}
