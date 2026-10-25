//! Plane's internal API (`/api/...`, `/auth/...`) over HTTP, from `thrust-compat-plane` (issue #44). The hermetic
//! `parity_` tests of that crate compare each row with a recording of the pinned Plane; this module holds what has to be
//! true on every deployed target: both composition roots mount the routes, the status codes the React client acts on
//! (research/0003 section 9, rows 2 and 3), and the sign-up and sign-in that the stock app's JSON auth client uses.
//!
//! Every test that asserts "X is accepted" is paired with the neighbouring "Y is refused" inside the same test.

use reqwest::StatusCode;
use serde_json::{json, Value};
use thrust_contract_tests::*;

use super::t;

const STRONG: &str = "Contract-Sample-Pa55word-Zebra-81";

fn new_email() -> String {
    format!("signup-{}@example.test", uuid::Uuid::new_v4().simple())
}

async fn post(t: &Target, path: &str, body: &Value) -> reqwest::Response {
    t.http.post(t.url(path)).json(body).send().await.unwrap()
}

/// Whether the response sets a live session cookie (a value, not a clearing), with the attributes that make it safe.
fn session_cookie(r: &reqwest::Response) -> Option<String> {
    let cookies = SetCookie::all(r);
    let c = SetCookie::find(&cookies, SESSION_COOKIE)?;
    if c.value.is_empty() {
        return None;
    }
    assert!(c.has_attr("Secure") && c.has_attr("HttpOnly"), "{c:?}");
    Some(c.value.clone())
}

/// S01. The boot request answers anyone, before there is a session, and says setup is done: a `false` would send the
/// visitor to Plane's instance-admin setup, which Thrust does not have.
#[tokio::test]
async fn compat_instances_answers_without_a_session_and_reports_setup_done() {
    let t = t();
    let r = t.http.get(t.url("/api/instances/")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["instance"]["is_setup_done"], true, "{body}");
    assert_eq!(body["config"]["is_email_password_enabled"], true, "{body}");
}

/// S02. Without a session the answer is a 401 whose body is a JSON object with no `id`: the React client treats any 401
/// as "sign in" and reads the signed-in state from the `id`. Controls: an unknown bearer token is also "no session", and
/// a real session is not refused with a 401, so the 401 is not the route's constant answer.
#[tokio::test]
async fn compat_users_me_without_a_session_is_401_json_with_no_id() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let url = t.url("/api/users/me/");

    for (what, req) in [
        ("no credentials", t.http.get(&url)),
        ("an unknown bearer token", t.http.get(&url).bearer_auth("not-a-session-token")),
    ] {
        let r = req.send().await.unwrap();
        assert_eq!(r.status(), 401, "{what}");
        let body: Value = r.json().await.unwrap();
        assert!(body.is_object(), "{what}: {body}");
        assert!(body.get("id").is_none(), "{what}: a signed-out answer must not carry an id: {body}");
    }

    let r = t.http.get(&url).bearer_auth(&f.admin_a.token).send().await.unwrap();
    assert_eq!(r.status(), 200, "a real session was treated as signed out");
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["id"], f.admin_a.id.0.to_string(), "{body}");
    assert_eq!(body["email"], f.admin_a.email, "{body}");
}

/// A new account, signed in through the compat sign-up: its email and the session token (accepted as a bearer token).
async fn signed_up(t: &Target) -> (String, String) {
    let email = new_email();
    let r = post(t, "/auth/sign-up/", &json!({"email": email, "password": STRONG})).await;
    assert_eq!(r.status(), 200);
    let token = session_cookie(&r).expect("a sign-up signs the new account in");
    (email, token)
}

async fn patch(t: &Target, token: &str, path: &str, body: &Value) -> reqwest::Response {
    t.http.patch(t.url(path)).bearer_auth(token).json(body).send().await.unwrap()
}

async fn get(t: &Target, token: &str, path: &str) -> Value {
    let r = t.http.get(t.url(path)).bearer_auth(token).send().await.unwrap();
    assert_eq!(r.status(), 200, "GET {path}");
    r.json().await.unwrap()
}

/// S02 and S10. A new account is itself: its email, a display name from the local part, UTC. A patch changes the names and
/// the time zone and nothing else, answers the long user, and a refused field refuses the whole patch (Plane validates
/// every field before it saves any). Controls: a name that is a URL, an unknown zone and a blank display name are each
/// refused with the field named, and what the refused patch carried does not land.
#[tokio::test]
async fn compat_a_user_patch_changes_names_and_zone_and_refuses_urls_unknown_zones_and_blanks() {
    let t = t();
    let (email, token) = signed_up(&t).await;
    let me = get(&t, &token, "/api/users/me/").await;
    assert_eq!(me["email"], email);
    assert_eq!(me["display_name"], email.split('@').next().unwrap());
    assert_eq!((me["first_name"].as_str(), me["user_timezone"].as_str()), (Some(""), Some("UTC")));
    assert!(me["id"].is_string() && me["username"].as_str().is_some_and(|u| u.len() == 32), "{me}");

    let r = patch(&t, &token, "/api/users/me/", &json!({"first_name": "  Journey ", "last_name": "Tester", "user_timezone": "Asia/Kolkata"})).await;
    assert_eq!(r.status(), 200);
    let long: Value = r.json().await.unwrap();
    assert_eq!((long["first_name"].as_str(), long["last_name"].as_str(), long["user_timezone"].as_str()), (Some("Journey"), Some("Tester"), Some("Asia/Kolkata")), "{long}");
    assert_eq!(long.as_object().unwrap().len(), 39, "the long user has every column of the pin's User but the password");
    assert!(long.get("avatar_url").is_none(), "the long user has no avatar_url (that is the short user's)");

    for (what, body, field, message) in [
        ("a URL in the first name", json!({"first_name": "see example.com"}), "first_name", "First name cannot contain a URL."),
        ("a URL in the last name", json!({"last_name": "http://example.test"}), "last_name", "Last name cannot contain a URL."),
        ("a blank display name", json!({"display_name": ""}), "display_name", "This field may not be blank."),
        ("an unknown zone", json!({"user_timezone": "Mars/Olympus"}), "user_timezone", "\"Mars/Olympus\" is not a valid choice."),
    ] {
        // a good field travels with the bad one, and must not land either
        let mut with_good = body.clone();
        with_good["avatar"] = json!("must-not-land");
        let r = patch(&t, &token, "/api/users/me/", &with_good).await;
        assert_eq!(r.status(), 400, "{what}");
        let got: Value = r.json().await.unwrap();
        assert_eq!(got, json!({ field: [message] }), "{what}");
    }
    let after = get(&t, &token, "/api/users/me/").await;
    assert_eq!(after["avatar"], "", "a refused patch wrote nothing");
    assert_eq!((after["first_name"].as_str(), after["user_timezone"].as_str()), (Some("Journey"), Some("Asia/Kolkata")), "and left what was there");

    // the pin ignores the fields it marks read only, instead of refusing them
    let r = patch(&t, &token, "/api/users/me/", &json!({"email": "someone@else.test", "is_bot": true, "username": "chosen"})).await;
    assert_eq!(r.status(), 200);
    let kept: Value = r.json().await.unwrap();
    assert_eq!((kept["email"].as_str(), kept["is_bot"].as_bool()), (Some(email.as_str()), Some(false)), "{kept}");

    // and without a session, nothing
    let r = t.http.patch(t.url("/api/users/me/")).json(&json!({"first_name": "x"})).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

/// S07 and S11. A profile has Plane's defaults; a patch sets the keys it names and keeps the others; a field that is the
/// wrong kind is refused with DRF's message and the profile is unchanged. Control: the same profile, unauthenticated, is a
/// 401, and another account's profile is its own.
#[tokio::test]
async fn compat_a_profile_has_the_defaults_and_a_patch_changes_only_what_it_names() {
    let t = t();
    let (_, token) = signed_up(&t).await;
    let (_, other) = signed_up(&t).await;
    let p = get(&t, &token, "/api/users/me/profile/").await;
    assert_eq!(p.as_object().unwrap().len(), 29, "{p}");
    assert_eq!((p["is_onboarded"].as_bool(), p["language"].as_str(), p["start_of_the_week"].as_i64()), (Some(false), Some("en"), Some(0)), "{p}");
    assert_eq!(p["onboarding_step"], json!({"profile_complete": false, "workspace_create": false, "workspace_invite": false, "workspace_join": false}));
    assert!(p["last_workspace_id"].is_null() && p["role"].is_null());

    let r = patch(&t, &token, "/api/users/me/profile/", &json!({"role": "developer"})).await;
    assert_eq!(r.status(), 200);
    let r = patch(&t, &token, "/api/users/me/profile/", &json!({"use_case": "Plan work", "start_of_the_week": "2", "has_marketing_email_consent": "yes"})).await;
    assert_eq!(r.status(), 200);
    let after = get(&t, &token, "/api/users/me/profile/").await;
    assert_eq!((after["role"].as_str(), after["use_case"].as_str()), (Some("developer"), Some("Plan work")), "the second patch kept the first's role: {after}");
    assert_eq!((after["start_of_the_week"].as_i64(), after["has_marketing_email_consent"].as_bool()), (Some(2), Some(true)), "DRF's choice and boolean spellings");

    for (what, body, field, message) in [
        ("a start of the week out of range", json!({"start_of_the_week": 9}), "start_of_the_week", "\"9\" is not a valid choice."),
        ("a boolean that is not one", json!({"is_onboarded": "maybe"}), "is_onboarded", "Must be a valid boolean."),
        ("a last workspace that is not an id", json!({"last_workspace_id": "nope"}), "last_workspace_id", "Must be a valid UUID."),
        ("a null theme", json!({"theme": null}), "theme", "This field may not be null."),
    ] {
        let mut with_good = body.clone();
        with_good["role"] = json!("must-not-land");
        let r = patch(&t, &token, "/api/users/me/profile/", &with_good).await;
        assert_eq!(r.status(), 400, "{what}");
        assert_eq!(r.json::<Value>().await.unwrap(), json!({ field: [message] }), "{what}");
    }
    assert_eq!(get(&t, &token, "/api/users/me/profile/").await["role"], "developer", "a refused patch wrote nothing");

    // an account's profile is its own
    assert!(get(&t, &other, "/api/users/me/profile/").await["role"].is_null());
    let r = t.http.get(t.url("/api/users/me/profile/")).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

/// S27 and X04. Plane sets these flags straight from the body: a boolean sets it, a missing key is `false`, and anything
/// else (null too) is its one 400. Controls: no session is a 401, and the flags read back through the profile.
#[tokio::test]
async fn compat_the_tour_and_onboarded_flags_follow_the_pins_rules() {
    let t = t();
    let (_, token) = signed_up(&t).await;
    for (path, key, flag) in [
        ("/api/users/me/tour-completed/", "is_tour_completed", "is_tour_completed"),
        ("/api/users/me/onboard/", "is_onboarded", "is_onboarded"),
    ] {
        let set = |body: Value| {
            let (t, token) = (&t, &token);
            async move { patch(t, token, path, &body).await }
        };
        let r = set(json!({ key: true })).await;
        assert_eq!(r.status(), 200, "{path}");
        assert_eq!(r.json::<Value>().await.unwrap(), json!({"message": "Updated successfully"}));
        assert_eq!(get(&t, &token, "/api/users/me/profile/").await[flag], true, "{path}");

        for bad in [json!({ key: "abc" }), json!({ key: null }), json!({ key: [true] })] {
            let r = set(bad.clone()).await;
            assert_eq!(r.status(), 400, "{path} {bad}");
            assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "Please provide valid detail"}));
        }
        assert_eq!(get(&t, &token, "/api/users/me/profile/").await[flag], true, "{path}: a refused body does not change it");

        // a missing key is false; Django's own spellings are booleans
        assert_eq!(set(json!({})).await.status(), 200);
        assert_eq!(get(&t, &token, "/api/users/me/profile/").await[flag], false, "{path}: no key sets false");
        assert_eq!(set(json!({ key: "t" })).await.status(), 200);
        assert_eq!(get(&t, &token, "/api/users/me/profile/").await[flag], true, "{path}: \"t\"");

        let r = t.http.patch(t.url(path)).json(&json!({ key: true })).send().await.unwrap();
        assert_eq!(r.status(), 401, "{path} without a session");
    }
}

/// S03. A token as JSON and as the CSRF cookie, to anyone. Two calls give two tokens.
#[tokio::test]
async fn compat_csrf_token_is_json_and_sets_the_csrf_cookie() {
    let t = t();
    let get = || async { t.http.get(t.url("/auth/get-csrf-token/")).send().await.unwrap() };
    let (a, b) = (get().await, get().await);
    assert_eq!(a.status(), 200);
    let cookies = SetCookie::all(&a);
    let cookie = SetCookie::find(&cookies, CSRF_COOKIE).expect("a csrf cookie").clone();
    let (a, b): (Value, Value) = (a.json().await.unwrap(), b.json().await.unwrap());
    let token = a["csrf_token"].as_str().expect("a csrf_token string");
    assert!(token.len() >= 32, "{token}");
    assert_eq!(token, cookie.value, "the body's token is the cookie's");
    assert_ne!(a["csrf_token"], b["csrf_token"], "a fresh token each time");
}

/// S04. Whether an account has the email, in any spelling, and the two refusals Plane has. Control: the answer is not
/// constant, an email nobody has is `false` and a seeded one `true`.
#[tokio::test]
async fn compat_email_check_tells_a_known_email_from_an_unknown_one_and_refuses_bad_input() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let check = |email: Value| {
        let t = &t;
        async move { post(t, "/auth/email-check/", &json!({ "email": email })).await }
    };

    for (what, email, existing) in [
        ("a seeded email", f.member_a.email.clone(), true),
        ("the same email in capitals, padded", format!("  {}  ", f.member_a.email.to_uppercase()), true),
        ("an email nobody has", new_email(), false),
    ] {
        let r = check(json!(email)).await;
        assert_eq!(r.status(), 200, "{what}");
        let body: Value = r.json().await.unwrap();
        assert_eq!(body, json!({"existing": existing, "status": "CREDENTIAL"}), "{what}");
    }

    for (what, body, code) in [
        ("no email", json!({}), 5010),
        ("an empty email", json!({"email": ""}), 5010),
        ("not an email", json!({"email": "not-an-email"}), 5005),
        ("not JSON at all", Value::Null, 5010),
    ] {
        let r = if body.is_null() {
            t.http.post(t.url("/auth/email-check/")).body("not json").send().await.unwrap()
        } else {
            post(&t, "/auth/email-check/", &body).await
        };
        assert_eq!(r.status(), 400, "{what}");
        let got: Value = r.json().await.unwrap();
        assert_eq!(got["error_code"], code, "{what}: {got}");
        assert!(got["error_message"].is_string(), "{what}: {got}");
    }
}

/// S05. A sign-up makes an account and signs it in: the session cookie works, the same email signs in afterwards in any
/// case. Controls: the same email again, a weak password and a malformed email are each refused, with Plane's codes, no
/// cookie, and without making or changing an account.
#[tokio::test]
async fn compat_sign_up_makes_an_account_that_is_signed_in_and_refuses_a_taken_email_or_weak_password() {
    let t = t();
    let email = new_email();
    let r = post(&t, "/auth/sign-up/", &json!({"email": email, "password": STRONG})).await;
    assert_eq!(r.status(), 200);
    let token = session_cookie(&r).expect("a sign-up signs the new account in");
    let body: Value = r.json().await.unwrap();
    assert_eq!(body, json!({"redirect_path": "/"}));

    // the cookie is a real session: it authenticates against a route that needs one
    let me = t.http.get(t.url(&format!("{SLICE_PREFIX}/auth/session"))).bearer_auth(&token).send().await.unwrap();
    assert_eq!(me.status(), 200, "the session the sign-up made does not authenticate");

    // and the account exists, however the email is spelled
    let r = post(&t, "/auth/email-check/", &json!({"email": email.to_uppercase()})).await;
    assert_eq!(r.json::<Value>().await.unwrap()["existing"], true);

    for (what, body, code) in [
        ("the same email again", json!({"email": email, "password": STRONG}), 5030),
        ("the same email in capitals", json!({"email": email.to_uppercase(), "password": STRONG}), 5030),
        ("a weak password", json!({"email": new_email(), "password": "password"}), 5021),
        ("a taken email and a weak password (taken is checked first)", json!({"email": email, "password": "password"}), 5030),
        ("not an email", json!({"email": "nope", "password": STRONG}), 5045),
        ("no fields", json!({}), 5040),
        ("an empty password", json!({"email": new_email(), "password": ""}), 5040),
    ] {
        let r = post(&t, "/auth/sign-up/", &body).await;
        assert_eq!(r.status(), 400, "{what}");
        assert!(session_cookie(&r).is_none(), "{what}: a refused sign-up set a session");
        let got: Value = r.json().await.unwrap();
        assert_eq!(got["error_code"], code, "{what}: {got}");
    }

    // nothing was made by the weak one: its email is still nobody's
    let weak = new_email();
    post(&t, "/auth/sign-up/", &json!({"email": weak, "password": "password"})).await;
    let r = post(&t, "/auth/email-check/", &json!({"email": weak})).await;
    assert_eq!(r.json::<Value>().await.unwrap()["existing"], false);
}

/// S06. A seeded user signs in with the right password, in any spelling of the email. Controls, each refused with
/// Plane's code and no cookie: a wrong password, an email nobody has, a malformed email, missing fields, and a
/// deactivated user (one answer for every refusal of a password, so the response does not say which).
#[tokio::test]
async fn compat_sign_in_signs_a_seeded_user_in_and_refuses_everyone_else_with_planes_codes() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let u = &f.member_a;

    for (what, email) in [("the seeded email", u.email.clone()), ("another spelling", format!(" {} ", u.email.to_uppercase()))] {
        let r = post(&t, "/auth/sign-in/", &json!({"email": email, "password": u.password})).await;
        assert_eq!(r.status(), 200, "{what}");
        let token = session_cookie(&r).unwrap_or_else(|| panic!("{what}: no session cookie"));
        assert_eq!(r.json::<Value>().await.unwrap(), json!({"redirect_path": "/"}), "{what}");
        let me = t.http.get(t.url(&format!("{SLICE_PREFIX}/auth/session"))).bearer_auth(&token).send().await.unwrap();
        assert_eq!(me.status(), 200, "{what}: the session does not authenticate");
    }

    let inactive = f.extra_member(&t).await;
    t.set_user_active(inactive.id, false).await;

    for (what, body, code) in [
        ("a wrong password", json!({"email": u.email, "password": "not the password at all"}), 5065),
        ("an email nobody has", json!({"email": new_email(), "password": STRONG}), 5060),
        ("not an email", json!({"email": "nope", "password": STRONG}), 5075),
        ("no fields", json!({}), 5070),
        ("an empty email", json!({"email": "", "password": STRONG}), 5070),
        ("a deactivated user with the right password", json!({"email": inactive.email, "password": inactive.password}), 5065),
    ] {
        let r = post(&t, "/auth/sign-in/", &body).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{what}");
        assert!(session_cookie(&r).is_none(), "{what}: a refused sign-in set a session");
        let got: Value = r.json().await.unwrap();
        assert_eq!(got["error_code"], code, "{what}: {got}");
    }
}

/// Where the browser goes after a sign-in or sign-up: the root, with `?next_path=` for a safe place and the bare root
/// for an unsafe one. The unsafe ones are the open-redirect attempts: a second slash, a scheme, a traversal, a backslash.
#[tokio::test]
async fn compat_a_safe_next_path_is_kept_and_an_unsafe_one_is_dropped() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let u = &f.member_a;
    for (what, next, redirect) in [
        ("a safe path", "/acme/projects", "/?next_path=/acme/projects"),
        ("a path with a query, encoded", "/a?b=c&d=e", "/?next_path=/a%3Fb%3Dc%26d%3De"),
        ("a second leading slash", "//evil.test/x", "/"),
        ("an absolute URL", "https://evil.test/x", "/"),
        ("a javascript URL", "javascript:alert(1)", "/"),
        ("a traversal", "/a/../b", "/"),
        ("a backslash", "/\\evil.test", "/"),
        ("no leading slash", "acme", "/"),
    ] {
        let r = post(&t, "/auth/sign-in/", &json!({"email": u.email, "password": u.password, "next_path": next})).await;
        assert_eq!(r.status(), 200, "{what}");
        assert_eq!(r.json::<Value>().await.unwrap(), json!({"redirect_path": redirect}), "{what}: {next}");
    }
}
