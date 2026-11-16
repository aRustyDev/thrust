//! The operator's way in, `/internal/operator/*` (Q-F7, issue #46), on every target: make an account with a temporary password, and
//! set a new password for one, for a person who cannot sign in. The target is started with the routes on and a token
//! (`scripts/contract.sh`); the hermetic tests state the switch, the token's length and the answers in detail
//! (`thrust-compat-plane/tests/operator.rs`, `thrust-core/tests/operator.rs`), and this is what has to be true wherever the routes
//! are mounted, through the Plane sign-in the stock app uses.
//!
//! Every test that asserts "X is allowed" is paired with the neighbouring "Y is refused" inside the same test.

use reqwest::StatusCode;
use serde_json::{json, Value};
use thrust_contract_tests::*;
use thrust_contracts::UserId;

use super::compat::{new_email, post, session_cookie, signed_up, STRONG};
use super::t;

const NEW: &str = "Contract-Changed-Pa55phrase-Quokka-27";

async fn sign_in(t: &Target, email: &str, password: &str) -> reqwest::Response {
    post(t, "/auth/sign-in/", &json!({"email": email, "password": password})).await
}

/// A sign-in that works: the session it makes, which is a live one.
async fn signed_in(t: &Target, email: &str, password: &str, what: &str) -> String {
    let r = sign_in(t, email, password).await;
    assert_eq!(r.status(), 200, "{what}");
    let token = session_cookie(&r).unwrap_or_else(|| panic!("{what}: no session cookie"));
    assert_eq!(t.me_status(&token).await, 200, "{what}: the session does not authenticate");
    token
}

/// A sign-in that is refused with Plane's one answer for a wrong password, and makes no session.
async fn refused(t: &Target, email: &str, password: &str, what: &str) {
    let r = sign_in(t, email, password).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{what}");
    assert!(session_cookie(&r).is_none(), "{what}: a refused sign-in set a session");
    let got: Value = r.json().await.unwrap();
    assert_eq!(got["error_code"], 5065, "{what}: {got}");
}

async fn exists(t: &Target, email: &str) -> bool {
    let r = post(t, "/auth/email-check/", &json!({"email": email})).await;
    r.json::<Value>().await.unwrap()["existing"].as_bool().expect("an existing flag")
}

/// A POST to one of the operator's routes with exactly the headers given (none of the operator's own).
async fn knock(t: &Target, route: &str, headers: &[(&str, &str)], email: &str, password: &str) -> reqwest::Response {
    let mut req = t.http.post(t.url(&format!("/internal/operator/{route}")));
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    req.json(&json!({"email": email, "password": password})).send().await.unwrap()
}

async fn user_id_of(t: &Target, token: &str) -> UserId {
    let me: Value = t.http.get(t.url("/api/users/me/")).bearer_auth(token).send().await.unwrap().json().await.unwrap();
    UserId(uuid::Uuid::parse_str(me["id"].as_str().expect("an id")).expect("a uuid"))
}

/// Nothing but the operator's token opens either route, and the two doors are two: the test routes' token opens nothing here, the
/// operator's opens no test route, and a person's own session is no way in. Each refused call changed nothing (an account that
/// does not exist is still nobody's, one that does still has its password and its session). Control: the same calls with the
/// operator's token are not refused, so the 401 is not the route's constant answer.
#[tokio::test]
async fn operator_routes_refuse_every_request_without_the_operators_token() {
    let t = t();
    let op = t.operator_token().to_string();
    let wrong = "0".repeat(op.len());
    let nobody = new_email();
    let (victim, session) = signed_up(&t).await;
    let bearer = format!("Bearer {session}");
    let cookie = format!("__Host-thrust_session={session}");

    let attempts: [(&str, Vec<(&str, &str)>); 7] = [
        ("no token", vec![]),
        ("a wrong token of the right length", vec![("x-operator-token", wrong.as_str())]),
        ("the test routes' token in the operator's header", vec![("x-operator-token", t.admin_token.as_str())]),
        ("the test routes' token in its own header", vec![("x-admin-token", t.admin_token.as_str())]),
        ("the operator's token in the test routes' header", vec![("x-admin-token", op.as_str())]),
        ("a session as a bearer token", vec![("authorization", bearer.as_str())]),
        ("a session as a cookie", vec![("cookie", cookie.as_str())]),
    ];
    for (what, headers) in &attempts {
        for (route, email) in [("users", nobody.as_str()), ("users/password", victim.as_str())] {
            let r = knock(&t, route, headers, email, NEW).await;
            assert_eq!(r.status(), 401, "{what} on {route}");
        }
    }
    // the other direction: the operator's token opens no test route
    let r = t.http.post(t.url("/internal/sweep")).header("x-admin-token", &op).send().await.unwrap();
    assert_eq!(r.status(), 401, "the operator's token on a test route");

    // control: nothing changed
    assert!(!exists(&t, &nobody).await, "an account was made without the token");
    assert_eq!(t.me_status(&session).await, 200, "the victim's session ended without the token");
    signed_in(&t, &victim, STRONG, "the victim's own password still signs in").await;
    refused(&t, &victim, NEW, "the password a refused reset carried").await;

    // control: the operator's token is not refused, on both routes, and only POST
    let r = t.operator_post("users", Some(&op), &nobody, NEW).await;
    assert_eq!(r.status(), 204, "the operator's token on create");
    let r = t.operator_post("users/password", Some(&op), &victim, NEW).await;
    assert_eq!(r.status(), 204, "the operator's token on reset");
    let r = t.http.get(t.url("/internal/operator/users")).header("x-operator-token", &op).send().await.unwrap();
    assert!(matches!(r.status().as_u16(), 404 | 405), "a GET is no way to make an account: {}", r.status());
}

/// The operator makes an account with a temporary password: no answer body, the email in any spelling is the account, and the
/// person signs in with the password and with no other. Controls: making it again is a 409, and the second password is no
/// password; a password that is too weak and an email that is not one are a 400 and make nothing.
#[tokio::test]
async fn operator_creates_an_account_that_signs_in_and_cannot_be_made_twice() {
    let t = t();
    let op = t.operator_token().to_string();
    let email = new_email();

    let r = t.operator_post("users", Some(&op), &format!(" {} ", email.to_uppercase()), STRONG).await;
    assert_eq!(r.status(), 204);
    assert!(r.bytes().await.unwrap().is_empty(), "no body");
    assert!(exists(&t, &email).await);
    signed_in(&t, &email, STRONG, "the temporary password").await;
    refused(&t, &email, NEW, "another password").await;

    let r = t.operator_post("users", Some(&op), &email, NEW).await;
    assert_eq!(r.status(), 409, "the email is taken");
    signed_in(&t, &email, STRONG, "the first password still signs in after a refused second make").await;
    refused(&t, &email, NEW, "the refused second password").await;

    for (what, who, password) in [
        ("a weak password", new_email(), "password123"),
        ("a short one", new_email(), "short"),
        ("an email that is not one", "not-an-email".to_string(), STRONG),
    ] {
        let r = t.operator_post("users", Some(&op), &who, password).await;
        assert_eq!(r.status(), 400, "{what}");
        // (the account check answers an error, not a flag, for an email that is not one: there is nothing to look up)
        if who.contains('@') {
            assert!(!exists(&t, &who).await, "{what}: an account was made");
        }
    }
}

/// A reset swaps the password of that account and no other, ends every session of it and no other, and leaves the account as
/// active as it was. The target person has three sessions; a bystander has one and the same password. Controls: a weak password
/// and an email with no account are refused and change nothing; the bystander's session and password are as they were; a
/// deactivated account stays deactivated, and finds the new password in place when it is active again.
#[tokio::test]
async fn operator_reset_swaps_the_password_ends_its_sessions_and_leaves_everyone_else_alone() {
    let t = t();
    let op = t.operator_token().to_string();

    let (target, first) = signed_up(&t).await;
    let second = signed_in(&t, &target, STRONG, "a second session").await;
    let third = signed_in(&t, &target, STRONG, "a third session").await;
    let (bystander, theirs) = signed_up(&t).await;
    for (what, s) in [("first", &first), ("second", &second), ("third", &third), ("the bystander's", &theirs)] {
        assert_eq!(t.me_status(s).await, 200, "control: the {what} session is live before the reset");
    }

    // refused resets change nothing
    let r = t.operator_post("users/password", Some(&op), &target, "password123").await;
    assert_eq!(r.status(), 400, "a weak password");
    let r = t.operator_post("users/password", Some(&op), &new_email(), NEW).await;
    assert_eq!(r.status(), 404, "an email with no account");
    assert_eq!(t.me_status(&first).await, 200, "a refused reset ended a session");
    signed_in(&t, &target, STRONG, "the old password after refused resets").await;

    let r = t.operator_post("users/password", Some(&op), &format!(" {} ", target.to_uppercase()), NEW).await;
    assert_eq!(r.status(), 204);
    assert!(r.bytes().await.unwrap().is_empty(), "no body");

    refused(&t, &target, STRONG, "the old password after the reset").await;
    for (what, s) in [("first", &first), ("second", &second), ("third", &third)] {
        assert_eq!(t.me_status(s).await, 401, "the {what} session of the reset account still stands");
    }
    let fresh = signed_in(&t, &target, NEW, "the new password").await;
    assert_ne!(fresh, first);

    assert_eq!(t.me_status(&theirs).await, 200, "the bystander's session ended with someone else's reset");
    signed_in(&t, &bystander, STRONG, "the bystander's own password, which is the old one the target had").await;
    refused(&t, &bystander, NEW, "the new password is not the bystander's").await;

    // a reset leaves an account as active as it was
    let (sleeper, session) = signed_up(&t).await;
    let id = user_id_of(&t, &session).await;
    t.set_user_active(id, false).await;
    let r = t.operator_post("users/password", Some(&op), &sleeper, NEW).await;
    assert_eq!(r.status(), 204, "a deactivated account can have its password set");
    refused(&t, &sleeper, NEW, "a password does not reactivate").await;
    t.set_user_active(id, true).await;
    signed_in(&t, &sleeper, NEW, "the new password, once the account is active").await;
    refused(&t, &sleeper, STRONG, "the old password").await;
}
