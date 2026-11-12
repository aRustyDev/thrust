//! `POST /auth/sign-out/` (X06, D-036): what the sign-out menu calls. The pin answers a 302 to the app's origin for every request,
//! a request with no session included; Thrust's copy of the client posts JSON and reads JSON (D-022), so this answers 200
//! `{"redirect_path": "/"}` and clears both cookies, whatever the request holds. The recorded row (`parity_x06_sign_out`) shows
//! the pin's side; these tests state what the recording cannot, which is what happens to the sessions.
//!
//! What must hold: the answer and the cleared cookies are the same for a request with no session, an unknown one, a live one and
//! one that has ended; the live session ends (the next request with it is a 401) and nobody else's does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use common::*;
use rstest::rstest;
use serde_json::{json, Value};

const A: &str = "signs-out@example.test";
const B: &str = "stays-in@example.test";
const GHOST: &str = "never-signed-in@example.test";

fn sign_out(router: &axum::Router, jar: &Jar, who: &str) -> Value {
    call(router, jar, who, "POST", "/auth/sign-out/", Value::Null)
}

fn me_status(router: &axum::Router, jar: &Jar, who: &str) -> u64 {
    call(router, jar, who, "GET", "/api/users/me/", Value::Null)["status"].as_u64().expect("a status")
}

/// Both cookies the answer sets, cleared: empty, expired at once, with the attributes that make them the `__Host-` ones.
fn assert_both_cleared(response: &Value) {
    let cookies: Vec<&str> = match &response["headers"]["set-cookie"] {
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        Value::String(s) => vec![s.as_str()],
        other => panic!("no cookies cleared: {other}"),
    };
    assert_eq!(cookies.len(), 2, "{cookies:?}");
    for name in ["__Host-thrust_session", "__Host-thrust_csrf"] {
        let c = cookies
            .iter()
            .find(|c| c.starts_with(&format!("{name}=")))
            .unwrap_or_else(|| panic!("{name}: {cookies:?}"));
        assert!(c.starts_with(&format!("{name}=;")), "{c}: the value is empty");
        assert!(
            c.contains("Max-Age=0") && c.contains("Path=/") && c.contains("Secure") && c.contains("HttpOnly"),
            "{c}"
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum Held {
    Nothing,
    AnUnknownCookie,
    ALiveSession,
    ASessionThatHasEnded,
}

#[rstest]
fn sign_out_answers_one_way_whatever_the_request_holds_and_ends_only_the_session_it_names(
    #[values(Held::Nothing, Held::AnUnknownCookie, Held::ALiveSession, Held::ASessionThatHasEnded)] held: Held,
) {
    let (router, _db, mut jar) = world(&[A, B]);
    let who = match held {
        Held::Nothing => "",
        Held::AnUnknownCookie => {
            jar.insert(jar_key(GHOST), "not-a-session-token".to_string());
            GHOST
        }
        Held::ALiveSession => A,
        Held::ASessionThatHasEnded => {
            assert_eq!(sign_out(&router, &jar, A)["status"], 200);
            A
        }
    };
    let live_before = me_status(&router, &jar, A);
    assert_eq!(live_before, if matches!(held, Held::ASessionThatHasEnded) { 401 } else { 200 }, "{held:?}");

    let r = sign_out(&router, &jar, who);
    assert_eq!(r["status"], 200, "{held:?}: {r}");
    assert_eq!(r["body"], json!({"redirect_path": "/"}), "{held:?}");
    assert_eq!(r["headers"]["cache-control"], "no-store", "an answer that clears a credential is not cached");
    assert_both_cleared(&r);

    // the session it named is over, and nobody else's is
    assert_eq!(
        me_status(&router, &jar, A),
        if matches!(held, Held::Nothing | Held::AnUnknownCookie) { 200 } else { 401 },
        "{held:?}"
    );
    assert_eq!(me_status(&router, &jar, B), 200, "{held:?}: another account's session is not touched");
}

/// The session token is the credential, not the account: an account signed in twice has two sessions, and signing out of one leaves
/// the other.
#[test]
fn only_the_session_that_signs_out_ends_when_an_account_has_two() {
    let (router, _db, mut jar) = world(&[A]);
    let raw = send(
        &router,
        &json!("sign-in"),
        &json!({"method": "POST", "path": "/auth/sign-in/", "body": {"email": A, "password": STRONG}}),
        &jar,
    );
    assert_eq!(raw["response"]["status"], 200, "{raw}");
    let second = session_token(&raw).expect("a second session");
    assert_ne!(Some(&second), jar.get(&jar_key(A)), "a sign-in makes a new session");
    jar.insert(jar_key("second-session@example.test"), second);

    assert_eq!(sign_out(&router, &jar, A)["status"], 200);
    assert_eq!(me_status(&router, &jar, A), 401);
    assert_eq!(me_status(&router, &jar, "second-session@example.test"), 200, "the other session of the account stays");
}
