//! `GET /auth/oidc/` and `GET /auth/oidc/callback/` (X10, D-040): the two routes behind the stock app's "Sign in with SSO" button.
//! There is no recording to compare them with (the pin's fork can only be recorded with an identity provider, and `parity_` needs the
//! pin's own answers), so these tests state what the routes must do, with a scripted provider that answers what it is told and counts
//! what it is asked.
//!
//! What must hold:
//! * a deployment with no provider is **dark**: `/api/instances/` says `is_oidc_enabled: false` (so the app draws no button), and the
//!   routes tell anyone who goes there by hand that none is configured (`OIDC_NOT_CONFIGURED`, 5300); with one it says `true`;
//! * `/auth/oidc/` is a 302 to the provider's authorization URL, with the `state` core made, `no-store`, no cookie, and the same
//!   answer whatever `?next_path=` says (the login does not carry it);
//! * the callback is a function of what the provider says and what the browser brings: an identity that is let in is a 302 to `/` with the
//!   two session cookies (a working session); every other outcome is a 302 to `/?error_code=&error_message=` with no cookie, `SIGNUP_DISABLED`
//!   (5015) for an identity Thrust will not let in (no verified email, no account, a deactivated one) and `OIDC_OAUTH_PROVIDER_ERROR` (5301) for
//!   a provider that rejects or cannot be reached and for a `state` or `code` that is missing, unknown or used;
//! * the provider is asked only when the `state` was one this server made and had not seen: never for an unknown, missing or replayed one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, Request};
use axum::{Extension, Router};
use common::*;
use rstest::rstest;
use serde_json::Value;
use thrust_core::admin_create_user;
use thrust_ports::{Database, OidcClaims, OidcError, OidcProvider, SystemScope};
use tower::ServiceExt;

const ISSUER: &str = "https://idp.example.test";
const MEMBER: &str = "member@example.test";
const STRANGER: &str = "stranger@example.test";
const SESSION: &str = "__Host-thrust_session";
const CSRF: &str = "__Host-thrust_csrf";

/// What the scripted provider vouches for when it is asked to exchange a code.
#[derive(Clone)]
enum Reply {
    Claims { email: Option<&'static str>, verified: bool },
    Rejected,
    Down,
}

/// A provider that answers what it is told, remembers the `state` of each authorization URL it made, and counts its exchanges.
struct Scripted {
    reply: Reply,
    states: Mutex<Vec<String>>,
    exchanges: AtomicUsize,
}

#[async_trait::async_trait]
impl OidcProvider for Scripted {
    async fn authorization_url(&self, state: &str, _nonce: &str, _code_verifier: &str) -> Result<String, OidcError> {
        self.states.lock().unwrap().push(state.to_string());
        Ok(format!("{ISSUER}/authorize?state={state}"))
    }

    async fn exchange(&self, _code: &str, _code_verifier: &str, _nonce: &str) -> Result<OidcClaims, OidcError> {
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        match &self.reply {
            Reply::Claims { email, verified } => Ok(OidcClaims {
                issuer: ISSUER.into(),
                subject: "sub-1".into(),
                email: email.map(str::to_string),
                email_verified: *verified,
            }),
            Reply::Rejected => Err(OidcError::Rejected("scripted".into())),
            Reply::Down => Err(OidcError::Provider("scripted".into())),
        }
    }
}

struct World {
    router: Router,
    db: Arc<thrust_adapters_memory::MemoryDatabase>,
    idp: Option<Arc<Scripted>>,
}

/// The compat routes over a memory world, with the provider given (or none: a dark deployment) and an active account for `MEMBER`.
fn world(reply: Option<Reply>) -> World {
    let idp =
        reply.map(|reply| Arc::new(Scripted { reply, states: Mutex::new(vec![]), exchanges: AtomicUsize::new(0) }));
    let (app, db) = bare_world_with(idp.clone().map(|p| p as Arc<dyn OidcProvider>));
    futures_executor::block_on(admin_create_user(&app, MEMBER, STRONG)).expect("the member's account");
    let router = thrust_compat_plane::routes().layer(Extension(app.clone()));
    World { router, db, idp }
}

impl World {
    fn deactivate_member(&self) {
        let id = user_id(&self.db, MEMBER);
        assert!(futures_executor::block_on(self.db.set_user_active(&SystemScope::mint(), id, false)).unwrap());
    }

    fn last_state(&self) -> String {
        self.idp.as_ref().expect("a provider").states.lock().unwrap().last().cloned().expect("a login was started")
    }

    fn exchanges(&self) -> usize {
        self.idp.as_ref().expect("a provider").exchanges.load(Ordering::SeqCst)
    }

    fn get(&self, uri: &str, session: Option<&str>) -> Answer {
        let mut req = Request::get(uri);
        if let Some(token) = session {
            req = req.header(header::COOKIE, format!("{SESSION}={token}"));
        }
        let resp = futures_executor::block_on(self.router.clone().oneshot(req.body(Body::empty()).unwrap())).unwrap();
        let (parts, body) = resp.into_parts();
        let bytes = futures_executor::block_on(axum::body::to_bytes(body, usize::MAX)).unwrap();
        Answer {
            status: parts.status.as_u16(),
            location: parts.headers.get(header::LOCATION).map(|v| v.to_str().unwrap().to_string()),
            cache_control: parts.headers.get(header::CACHE_CONTROL).map(|v| v.to_str().unwrap().to_string()),
            cookies: parts
                .headers
                .get_all(header::SET_COOKIE)
                .iter()
                .map(|v| v.to_str().unwrap().to_string())
                .collect(),
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }
}

struct Answer {
    status: u16,
    location: Option<String>,
    cache_control: Option<String>,
    cookies: Vec<String>,
    body: Value,
}

impl Answer {
    fn cookie(&self, name: &str) -> Option<&String> {
        self.cookies.iter().find(|c| c.starts_with(&format!("{name}=")))
    }
}

fn refused_location(code: u16, name: &str) -> String {
    format!("/?error_code={code}&error_message={name}")
}

// --- a deployment with no provider is dark -----------------------------------------------------------------------------------------

/// With no provider the app must draw no SSO button and the routes must say why they do nothing; with one the button is drawn. The flag
/// is the only thing that ties the button to the routes (`instance.rs`), so a control here is the pair: both worlds, both answers.
#[test]
fn the_sso_button_is_drawn_exactly_when_a_provider_is_configured() {
    for (provider, want) in [(None, false), (Some(Reply::Rejected), true), (Some(Reply::Down), true)] {
        let w = world(provider.clone());
        let a = w.get("/api/instances/", None);
        assert_eq!(a.status, 200);
        let config = &a.body["config"];
        assert_eq!(config["is_oidc_enabled"], want, "provider configured: {}", provider.is_some());
        // what the app shows besides: the password, and no other method (the sign-in journey holds this on a browser)
        assert_eq!(config["is_email_password_enabled"], true);
        for off in [
            "is_google_enabled",
            "is_github_enabled",
            "is_gitlab_enabled",
            "is_gitea_enabled",
            "is_magic_login_enabled",
            "is_smtp_configured",
        ] {
            assert_eq!(config[off], false, "{off}");
        }
    }
}

#[rstest]
fn a_deployment_with_no_provider_tells_whoever_comes_by_hand_that_none_is_configured(
    #[values(
        "/auth/oidc/",
        "/auth/oidc/?next_path=/ws/",
        "/auth/oidc/callback/",
        "/auth/oidc/callback/?code=c&state=s",
        "/auth/oidc/callback/?error=access_denied&state=s"
    )]
    uri: &str,
) {
    let w = world(None);
    let a = w.get(uri, None);
    assert_eq!(a.status, 302, "{uri}");
    assert_eq!(a.location, Some(refused_location(5300, "OIDC_NOT_CONFIGURED")), "{uri}");
    assert!(a.cookies.is_empty(), "{uri}: a dark deployment sets no cookie");
    assert_eq!(a.cache_control.as_deref(), Some("no-store"), "{uri}");
}

// --- the start ---------------------------------------------------------------------------------------------------------------------

#[rstest]
fn the_start_sends_the_browser_to_the_provider_and_reads_no_next_path(
    #[values(
        "/auth/oidc/",
        "/auth/oidc/?next_path=/ws/",
        "/auth/oidc/?next_path=https://evil.example/",
        "/auth/oidc/?next_path="
    )]
    uri: &str,
) {
    let w = world(Some(Reply::Rejected));
    let a = w.get(uri, None);
    assert_eq!(a.status, 302, "{uri}");
    // the provider's URL with the state core made, and nothing of the caller's
    assert_eq!(a.location, Some(format!("{ISSUER}/authorize?state={}", w.last_state())), "{uri}");
    assert_eq!(a.cache_control.as_deref(), Some("no-store"));
    assert!(a.cookies.is_empty(), "a start sets no cookie");
    assert_eq!(w.last_state().len(), 64, "256 random bits, as hex");
    // each start is its own login
    w.get(uri, None);
    let states = w.idp.as_ref().unwrap().states.lock().unwrap().clone();
    assert_eq!(states.len(), 2);
    assert_ne!(states[0], states[1], "a state is made per login");
}

// --- the callback ------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Who {
    /// A verified email of the active account.
    Member,
    DeactivatedMember,
    /// A verified email nobody has an account for.
    Stranger,
    UnverifiedMember,
    NoEmail,
    Rejected,
    Down,
}

const WHO: [Who; 7] =
    [Who::Member, Who::DeactivatedMember, Who::Stranger, Who::UnverifiedMember, Who::NoEmail, Who::Rejected, Who::Down];

#[derive(Clone, Copy, Debug)]
enum Query {
    /// The `code` and the `state` that `/auth/oidc/` made.
    Valid,
    /// A `state` this server never made.
    UnknownState,
    /// `Valid`, twice: the second must find nothing.
    Replayed,
    NoCode,
    NoState,
    /// What a provider sends when the person said no: `?error=access_denied&state=...`.
    ProviderError,
    Nothing,
}

const QUERIES: [Query; 7] = [
    Query::Valid,
    Query::UnknownState,
    Query::Replayed,
    Query::NoCode,
    Query::NoState,
    Query::ProviderError,
    Query::Nothing,
];

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Outcome {
    Session,
    Refused(u16, &'static str),
}

const SIGNUP_DISABLED: Outcome = Outcome::Refused(5015, "SIGNUP_DISABLED");
const PROVIDER_ERROR: Outcome = Outcome::Refused(5301, "OIDC_OAUTH_PROVIDER_ERROR");

fn reply_for(who: Who) -> Reply {
    match who {
        Who::Member | Who::DeactivatedMember => Reply::Claims { email: Some(MEMBER), verified: true },
        Who::Stranger => Reply::Claims { email: Some(STRANGER), verified: true },
        Who::UnverifiedMember => Reply::Claims { email: Some(MEMBER), verified: false },
        Who::NoEmail => Reply::Claims { email: None, verified: true },
        Who::Rejected => Reply::Rejected,
        Who::Down => Reply::Down,
    }
}

/// The expected table: what a callback with a state this server made, and a code, comes to for each thing the provider can say.
fn on_a_good_state(who: Who) -> Outcome {
    match who {
        Who::Member => Outcome::Session,
        Who::DeactivatedMember | Who::Stranger | Who::UnverifiedMember | Who::NoEmail => SIGNUP_DISABLED,
        Who::Rejected | Who::Down => PROVIDER_ERROR,
    }
}

/// The answer must be the outcome, with every part of it: for a session the redirect, the two cookies and a session that works; for a
/// refusal the redirect with the code the app reads, and no cookie.
fn check(w: &World, a: &Answer, want: Outcome, case: &str) {
    assert_eq!(a.status, 302, "{case}");
    assert_eq!(a.cache_control.as_deref(), Some("no-store"), "{case}");
    match want {
        Outcome::Session => {
            assert_eq!(a.location.as_deref(), Some("/"), "{case}");
            let session = a.cookie(SESSION).unwrap_or_else(|| panic!("{case}: no session cookie"));
            assert!(a.cookie(CSRF).is_some(), "{case}: no csrf cookie");
            for c in &a.cookies {
                assert!(
                    c.contains("; Secure") && c.contains("; HttpOnly") && c.contains("; SameSite=Lax"),
                    "{case}: {c}"
                );
                assert!(c.contains("; Path=/") && !c.contains("Domain"), "{case}: {c}");
            }
            let token = session.trim_start_matches(&format!("{SESSION}=")).split(';').next().unwrap().to_string();
            assert_eq!(token.len(), 64, "{case}");
            let me = w.get("/api/users/me/", Some(&token));
            assert_eq!(me.status, 200, "{case}: the cookie the callback set is a session");
            assert_eq!(me.body["email"], MEMBER, "{case}");
        }
        Outcome::Refused(code, name) => {
            assert_eq!(a.location, Some(refused_location(code, name)), "{case}");
            assert!(a.cookies.is_empty(), "{case}: a refused login sets no cookie");
        }
    }
}

/// Every thing the provider can say, by every shape of callback the browser can bring: 49 worlds, each against the expected table. The
/// provider is asked exactly when the state was a good one (once, and not again for a replay).
#[test]
fn every_callback_comes_to_the_outcome_the_table_says() {
    let (mut cases, mut sessions, mut signup_disabled, mut provider_errors) = (0, 0, 0, 0);
    for who in WHO {
        for query in QUERIES {
            let case = format!("{who:?} x {query:?}");
            let w = world(Some(reply_for(who)));
            if matches!(who, Who::DeactivatedMember) {
                w.deactivate_member();
            }
            let start = w.get("/auth/oidc/", None);
            assert_eq!(start.status, 302, "{case}: start");
            let state = w.last_state();
            let uri = match query {
                Query::Valid | Query::Replayed => format!("/auth/oidc/callback/?code=code-1&state={state}"),
                Query::UnknownState => format!("/auth/oidc/callback/?code=code-1&state={}", "0".repeat(64)),
                Query::NoCode => format!("/auth/oidc/callback/?state={state}"),
                Query::NoState => "/auth/oidc/callback/?code=code-1".to_string(),
                Query::ProviderError => format!("/auth/oidc/callback/?error=access_denied&state={state}"),
                Query::Nothing => "/auth/oidc/callback/".to_string(),
            };
            let (first, asked) = match query {
                Query::Valid | Query::Replayed => (on_a_good_state(who), 1),
                _ => (PROVIDER_ERROR, 0),
            };
            let a = w.get(&uri, None);
            check(&w, &a, first, &case);
            if matches!(query, Query::Replayed) {
                // the state was taken by the first callback, whatever came of it: the second finds nothing, and the provider is not asked again
                let again = w.get(&uri, None);
                check(&w, &again, PROVIDER_ERROR, &format!("{case} (replayed)"));
            }
            assert_eq!(w.exchanges(), asked, "{case}: how often the provider was asked");
            cases += 1;
            match first {
                Outcome::Session => sessions += 1,
                Outcome::Refused(5015, _) => signup_disabled += 1,
                Outcome::Refused(..) => provider_errors += 1,
            }
        }
    }
    // control: a matrix that reached nothing would pass; this one reached each kind of outcome, in the numbers the table gives
    assert_eq!(cases, 49);
    assert_eq!((sessions, signup_disabled), (2, 8), "one identity is let in, by two of the seven shapes of callback");
    assert_eq!(provider_errors, 49 - 2 - 8);
}

/// The first login of an account links the provider's subject, and the second goes in through the link: the stored login is used up
/// either way, so a second callback with the same code is a refusal and not a second session.
#[test]
fn a_login_is_one_session_and_its_state_is_not_good_for_another() {
    let w = world(Some(Reply::Claims { email: Some(MEMBER), verified: true }));
    w.get("/auth/oidc/", None);
    let state = w.last_state();
    let uri = format!("/auth/oidc/callback/?code=c&state={state}");
    let first = w.get(&uri, None);
    check(&w, &first, Outcome::Session, "first");
    let second = w.get(&uri, None);
    check(&w, &second, PROVIDER_ERROR, "second");
    // a new login with the same subject goes in again (through the link made by the first)
    w.get("/auth/oidc/", None);
    let uri = format!("/auth/oidc/callback/?code=c&state={}", w.last_state());
    check(&w, &w.get(&uri, None), Outcome::Session, "third");
    assert_eq!(w.exchanges(), 2);
}
