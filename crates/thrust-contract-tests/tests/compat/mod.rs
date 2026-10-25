//! Plane's internal API (`/api/...`, `/auth/...`) over HTTP, from `thrust-compat-plane` (issue #44). The hermetic
//! `parity_` tests of that crate compare each row with a recording of the pinned Plane; this module holds what has to be
//! true on every deployed target: both composition roots mount the routes, the status codes the React client acts on
//! (research/0003 section 9, rows 2 and 3), and the sign-up and sign-in that the stock app's JSON auth client uses.
//!
//! Every test that asserts "X is accepted" is paired with the neighbouring "Y is refused" inside the same test.

use reqwest::StatusCode;
use serde_json::{json, Value};
use thrust_contract_tests::*;
use thrust_contracts::{default_project_identifier, ProjectId, ProjectNetwork, Role, SeedFixture, SeedMembership, SeedProject, SeedProjectDetails, SeedWorkspace, WorkspaceId, WorkspaceSlug};

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

async fn post_as(t: &Target, token: &str, path: &str, body: &Value) -> reqwest::Response {
    t.http.post(t.url(path)).bearer_auth(token).json(body).send().await.unwrap()
}

/// A slug no earlier run of the suite used: a deployed target keeps its workspaces.
fn new_slug() -> String {
    format!("contract-{}", uuid::Uuid::new_v4().simple())
}

/// S13: whether the slug is free.
async fn slug_is_free(t: &Target, token: &str, slug: &str) -> bool {
    get(t, token, &format!("/api/workspace-slug-check/?slug={slug}")).await["status"].as_bool().expect("a status")
}

/// S08 to S09, S12 to S16 and S18, with S01 after them. A person makes a workspace and is its admin, and it is theirs
/// alone: it is in their list and the slug check calls its slug taken; its member list is them; their own member row says
/// admin. Controls, each paired with the person it is not: a second account has no workspace, is refused the member list
/// and the project roles of this one (with Plane's two different 403 bodies), gets the blank member row Plane answers a
/// stranger, and cannot take the slug; nobody without a session is let in.
#[tokio::test]
async fn compat_a_workspace_is_made_by_its_creator_who_is_its_admin_and_no_one_else_has_it() {
    let t = t();
    let (email, token) = signed_up(&t).await;
    let (_, other) = signed_up(&t).await;
    let slug = new_slug();
    let me = get(&t, &token, "/api/users/me/").await;

    assert_eq!(get(&t, &token, "/api/users/me/workspaces/invitations/").await, json!([]));
    assert_eq!(get(&t, &token, "/api/users/me/workspaces/").await, json!([]));
    let settings = get(&t, &token, "/api/users/me/settings/").await;
    assert_eq!((settings["workspace"]["fallback_workspace_slug"].clone(), settings["workspace"]["invites"].clone()), (Value::Null, json!(0)), "{settings}");
    assert!(slug_is_free(&t, &token, &slug).await);

    let r = post_as(&t, &token, "/api/workspaces/", &json!({"name": "Contract Workspace", "slug": slug, "organization_size": "2-10"})).await;
    assert_eq!(r.status(), 201);
    let made: Value = r.json().await.unwrap();
    assert_eq!(made.as_object().unwrap().len(), 17, "{made}");
    assert_eq!((made["slug"].as_str(), made["name"].as_str(), made["role"].as_i64(), made["total_members"].as_i64()), (Some(slug.as_str()), Some("Contract Workspace"), Some(20), Some(1)), "{made}");
    assert_eq!((made["owner"].clone(), made["created_by"].clone(), made["organization_size"].clone()), (me["id"].clone(), me["id"].clone(), json!("2-10")), "{made}");

    // theirs: listed, taken, the first one to open
    let listed = get(&t, &token, "/api/users/me/workspaces/").await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert_eq!((listed[0]["id"].clone(), listed[0]["role"].clone()), (made["id"].clone(), json!(20)));
    assert!(!slug_is_free(&t, &token, &slug).await, "a made workspace's slug is taken");
    let settings = get(&t, &token, "/api/users/me/settings/").await;
    assert_eq!(settings["workspace"]["fallback_workspace_slug"], json!(slug), "{settings}");
    assert_eq!(get(&t, &token, "/api/instances/").await["instance"]["workspaces_exist"], true, "S01 after the first workspace");

    // what it says about its creator
    let member = get(&t, &token, &format!("/api/workspaces/{slug}/workspace-members/me/")).await;
    assert_eq!((member["role"].as_i64(), member["is_active"].as_bool(), member["company_role"].as_str()), (Some(20), Some(true), Some("")), "{member}");
    let members = get(&t, &token, &format!("/api/workspaces/{slug}/members/")).await;
    assert_eq!(members.as_array().unwrap().len(), 1, "{members}");
    assert_eq!((members[0]["role"].clone(), members[0]["member"]["id"].clone(), members[0]["member"]["email"].clone()), (json!(20), me["id"].clone(), json!(email)), "{members}");
    assert_eq!(get(&t, &token, &format!("/api/users/me/workspaces/{slug}/project-roles/")).await, json!({}), "a new workspace has no project");

    // not theirs: nothing listed, nothing readable, nothing to take
    assert_eq!(get(&t, &other, "/api/users/me/workspaces/").await, json!([]));
    let r = t.http.get(t.url(&format!("/api/workspaces/{slug}/members/"))).bearer_auth(&other).send().await.unwrap();
    assert_eq!(r.status(), 403);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "You don't have the required permissions."}));
    let r = t.http.get(t.url(&format!("/api/users/me/workspaces/{slug}/project-roles/"))).bearer_auth(&other).send().await.unwrap();
    assert_eq!(r.status(), 403);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"detail": "You do not have permission to perform this action."}));
    let blank = get(&t, &other, &format!("/api/workspaces/{slug}/workspace-members/me/")).await;
    assert_eq!((blank["is_active"].clone(), blank["member"].clone(), blank["role"].clone()), (json!(false), Value::Null, Value::Null), "{blank}");
    let r = post_as(&t, &other, "/api/workspaces/", &json!({"name": "Mine now", "slug": slug})).await;
    assert_eq!(r.status(), 400);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"slug": ["Workspace with this slug already exists."]}));
    assert_eq!(get(&t, &other, "/api/users/me/workspaces/").await, json!([]), "the refused creation made nothing");

    // nobody without a session
    for path in [
        "/api/users/me/workspaces/".to_string(),
        "/api/users/me/workspaces/invitations/".to_string(),
        "/api/users/me/settings/".to_string(),
        format!("/api/workspace-slug-check/?slug={slug}"),
        format!("/api/workspaces/{slug}/workspace-members/me/"),
        format!("/api/workspaces/{slug}/members/"),
        format!("/api/users/me/workspaces/{slug}/project-roles/"),
    ] {
        let r = t.http.get(t.url(&path)).send().await.unwrap();
        assert_eq!(r.status(), 401, "GET {path}");
    }
    let r = t.http.post(t.url("/api/workspaces/")).json(&json!({"name": "x", "slug": new_slug()})).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

/// S14, the rules a workspace must satisfy. Each refusal has Plane's status and body, and makes nothing: the slug it named
/// is still free afterwards. Controls: a slug the pin reserves reads as taken before and after, and the same name and slug
/// without the fault is made.
#[tokio::test]
async fn compat_a_workspace_that_breaks_a_rule_is_refused_with_planes_message_and_makes_nothing() {
    let t = t();
    let (_, token) = signed_up(&t).await;
    let url_name = new_slug();
    let no_alnum = new_slug();
    for (what, body, status, want) in [
        ("no fields", json!({}), 400, json!({"error": "Both name and slug are required"})),
        ("no slug", json!({"name": "No Slug"}), 400, json!({"error": "Both name and slug are required"})),
        ("a name that is too long", json!({"name": "x".repeat(81), "slug": new_slug()}), 400, json!({"error": "The maximum length for name is 80 and for slug is 48"})),
        ("a slug that is too long", json!({"name": "Long Slug", "slug": "s".repeat(49)}), 400, json!({"error": "The maximum length for name is 80 and for slug is 48"})),
        ("a URL in the name", json!({"name": "see example.com", "slug": url_name}), 400, json!({"error": "Name cannot contain a URL"})),
        ("a name with no letter or number", json!({"name": "!!!", "slug": no_alnum}), 400, json!({"name": ["Name must contain at least one letter or number"]})),
        ("a reserved slug", json!({"name": "Reserved", "slug": "onboarding"}), 400, json!({"slug": ["Slug is not valid"]})),
        ("a slug with a space", json!({"name": "Spaced", "slug": "a b"}), 400, json!({"slug": ["Enter a valid \"slug\" consisting of letters, numbers, underscores or hyphens."]})),
    ] {
        let r = post_as(&t, &token, "/api/workspaces/", &body).await;
        assert_eq!(r.status(), status, "{what}");
        assert_eq!(r.json::<Value>().await.unwrap(), want, "{what}");
    }
    assert_eq!(get(&t, &token, "/api/users/me/workspaces/").await, json!([]), "no refusal made a workspace");
    for slug in [&url_name, &no_alnum] {
        assert!(slug_is_free(&t, &token, slug).await, "a refused creation took {slug}");
    }
    // a reserved slug is not free, and stays so
    assert!(!slug_is_free(&t, &token, "onboarding").await);

    // control: without the fault, the same request is made
    let r = post_as(&t, &token, "/api/workspaces/", &json!({"name": "see example", "slug": url_name})).await;
    assert_eq!(r.status(), 201);
    assert!(!slug_is_free(&t, &token, &url_name).await);
}

/// The nine routes the app reads when it opens a workspace and its home page (S17, S19 to S26), by the path after the slug and
/// whether Plane's 403 for them says `error` (its decorator) or `detail` (a permission class).
const OPENING_ROUTES: [(&str, bool); 9] = [
    ("projects/", false),
    ("user-favorites/", false),
    ("states/", true),
    ("sidebar-preferences/", false),
    ("user-properties/", true),
    ("users/notifications/unread/", false),
    ("home-preferences/", false),
    ("quick-links/", false),
    ("recent-visits/", false),
];

/// S17 and S19 to S26 for a member who has just made a workspace. Each answers 200: the stubs (S19, S20, S23, S25, S26) with
/// what the pin gives a member who has none, the sidebar with its seven entries and the pin's pins and places, the home page
/// with its three widgets (the last made first), and the list settings with the pin's defaults; asking again changes nothing
/// (the list settings keep their id), and a second member of the workspace's own rows are not the first's. Controls: a stranger
/// is refused every row with the body Plane's guard gives it, and nobody without a session is let in.
#[tokio::test]
async fn compat_opening_a_workspace_reads_the_sidebar_the_home_page_and_the_list_settings_once_and_refuses_everyone_else() {
    let t = t();
    let (_, token) = signed_up(&t).await;
    let (_, stranger) = signed_up(&t).await;
    let slug = new_slug();
    let made = post_as(&t, &token, "/api/workspaces/", &json!({"name": "Opening", "slug": slug})).await;
    assert_eq!(made.status(), 201);
    let made: Value = made.json().await.unwrap();
    let path = |tail: &str| format!("/api/workspaces/{slug}/{tail}");

    for tail in ["projects/", "user-favorites/", "states/", "quick-links/", "recent-visits/", "recent-visits/?entity_name=project"] {
        assert_eq!(get(&t, &token, &path(tail)).await, json!([]), "{tail}");
    }
    assert_eq!(
        get(&t, &token, &path("users/notifications/unread/")).await,
        json!({"total_unread_notifications_count": 0, "mention_unread_notifications_count": 0})
    );

    let sidebar = get(&t, &token, &path("sidebar-preferences/")).await;
    let pins: Vec<(String, bool, f64)> = {
        let mut v: Vec<_> = sidebar.as_object().unwrap().iter().map(|(k, p)| (k.clone(), p["is_pinned"].as_bool().unwrap(), p["sort_order"].as_f64().unwrap())).collect();
        v.sort_by(|a, b| a.2.total_cmp(&b.2));
        v
    };
    let want: Vec<(String, bool, f64)> =
        [("views", false), ("active_cycles", false), ("analytics", false), ("drafts", true), ("your_work", true), ("archives", false), ("stickies", true)]
            .iter()
            .enumerate()
            .map(|(i, (k, pinned))| ((*k).to_string(), *pinned, 65535.0 + 10000.0 * i as f64))
            .collect();
    assert_eq!(pins, want, "{sidebar}");

    let home = get(&t, &token, &path("home-preferences/")).await;
    let widgets: Vec<(&str, f64)> = home.as_array().unwrap().iter().map(|w| (w["key"].as_str().unwrap(), w["sort_order"].as_f64().unwrap())).collect();
    assert_eq!(widgets, [("my_stickies", 997.0), ("recents", 998.0), ("quick_links", 999.0)], "{home}");

    let props = get(&t, &token, &path("user-properties/")).await;
    assert_eq!((props["workspace"].clone(), props["navigation_project_limit"].clone(), props["navigation_control_preference"].clone()), (made["id"].clone(), json!(10), json!("ACCORDION")), "{props}");
    assert_eq!(props["display_filters"]["display_filters"]["layout"], "list", "{props}");

    // again: the same rows, not new ones
    assert_eq!(get(&t, &token, &path("sidebar-preferences/")).await, sidebar);
    assert_eq!(get(&t, &token, &path("home-preferences/")).await, home);
    assert_eq!(get(&t, &token, &path("user-properties/")).await["id"], props["id"], "the list settings were made again");

    // a stranger is refused every row, each with its own guard's body; nobody without a session is let in
    for (tail, class) in OPENING_ROUTES {
        let r = t.http.get(t.url(&path(tail))).bearer_auth(&stranger).send().await.unwrap();
        assert_eq!(r.status(), 403, "a stranger on {tail}");
        let want = if class {
            json!({"detail": "You do not have permission to perform this action."})
        } else {
            json!({"error": "You don't have the required permissions."})
        };
        assert_eq!(r.json::<Value>().await.unwrap(), want, "a stranger on {tail}");
        let r = t.http.get(t.url(&path(tail))).send().await.unwrap();
        assert_eq!(r.status(), 401, "no session on {tail}");
    }
}

/// S17 for each way of being in a workspace. A workspace admin (the person who made it) lists every project, a secret one they
/// are not in among them, with `member_role` null. A member of a seeded workspace lists the projects they are in and the public
/// ones, so a member of the project sees it with their role number and a member who is not in it sees it with `member_role`
/// null, and neither sees a secret project they are not in. Controls: the second kind of project is left out of the lists that
/// must not have it (the secret project is in the admin's and not in the members').
#[tokio::test]
async fn compat_a_workspace_admin_lists_every_project_and_a_member_only_their_own_and_the_public_ones() {
    let t = t();
    // the admin: made the workspace, so is its admin, and a secret project that nobody is in is in it
    let (_, admin) = signed_up(&t).await;
    let slug = new_slug();
    let made: Value = post_as(&t, &admin, "/api/workspaces/", &json!({"name": "Listed", "slug": slug})).await.json().await.unwrap();
    let ws = WorkspaceId(made["id"].as_str().unwrap().parse().unwrap());
    let secret = ProjectId(uuid::Uuid::new_v4());
    let seeded = SeedFixture {
        workspaces: vec![SeedWorkspace { id: ws, slug: WorkspaceSlug::new(&slug) }],
        projects: vec![SeedProject { id: secret, workspace_id: ws, name: "Secret".into() }],
        project_details: vec![SeedProjectDetails { identifier: Some("SEC".into()), network: ProjectNetwork::Secret, ..SeedProjectDetails::of(secret) }],
        ..SeedFixture::default()
    };
    assert_eq!(t.admin_post("/internal/seed", &serde_json::to_value(&seeded).unwrap()).await.status(), 204);
    let listed = get(&t, &admin, &format!("/api/workspaces/{slug}/projects/")).await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    let p = &listed[0];
    assert_eq!((p["id"].clone(), p["identifier"].clone(), p["network"].clone(), p["member_role"].clone()), (json!(secret.0), json!("SEC"), json!(0), Value::Null), "{p}");
    assert_eq!(p.as_object().unwrap().len(), 21, "{p}");

    // the members: a seeded workspace whose members are all members of it, one project they are in and the secret one they are not
    let f = Fixture::seed(&t).await;
    let hidden = ProjectId(uuid::Uuid::new_v4());
    let seeded = SeedFixture {
        workspaces: vec![SeedWorkspace { id: f.ws_a, slug: WorkspaceSlug::new(&f.a.slug) }],
        projects: vec![SeedProject { id: hidden, workspace_id: f.ws_a, name: "Hidden".into() }],
        project_details: vec![SeedProjectDetails { network: ProjectNetwork::Secret, ..SeedProjectDetails::of(hidden) }],
        memberships: vec![SeedMembership { project_id: hidden, workspace_id: f.ws_a, user_id: f.admin_a.id, role: Role::Admin }],
        ..SeedFixture::default()
    };
    assert_eq!(t.admin_post("/internal/seed", &serde_json::to_value(&seeded).unwrap()).await.status(), 204);
    let path = format!("/api/workspaces/{}/projects/", f.a.slug);
    for (who, role) in [(&f.member_a, json!(15)), (&f.guest_a, json!(5)), (&f.admin_a, json!(20))] {
        let listed = get(&t, &who.token, &path).await;
        let names: Vec<&str> = listed.as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
        // admin_a is in the secret project too (and is a MEMBER of the workspace, as seeded workspace members are): theirs is listed
        let hidden_to_them = !std::ptr::eq(who, &f.admin_a);
        assert_eq!(names.contains(&"Hidden"), !hidden_to_them, "{names:?}");
        let a = listed.as_array().unwrap().iter().find(|p| p["name"] == "A").expect("project A is listed");
        assert_eq!((a["member_role"].clone(), a["network"].clone()), (role, json!(2)), "{a}");
        assert_eq!(a["identifier"], json!(default_project_identifier(f.a.project)), "{a}");
    }
    // a member of the workspace who is in no project still sees the public one, with no role
    let outsider = get(&t, &f.outsider_a.token, &path).await;
    assert_eq!(outsider.as_array().unwrap().len(), 1, "{outsider}");
    assert_eq!((outsider[0]["name"].clone(), outsider[0]["member_role"].clone()), (json!("A"), Value::Null), "{outsider}");
}
