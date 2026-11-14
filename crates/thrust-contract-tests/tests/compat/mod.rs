//! Plane's internal API (`/api/...`, `/auth/...`) over HTTP, from `thrust-compat-plane` (issue #44). The hermetic
//! `parity_` tests of that crate compare each row with a recording of the pinned Plane; this module holds what has to be
//! true on every deployed target: both composition roots mount the routes, the status codes the React client acts on
//! (research/0003 section 9, rows 2 and 3), and the sign-up and sign-in that the stock app's JSON auth client uses.
//!
//! Every test that asserts "X is accepted" is paired with the neighbouring "Y is refused" inside the same test.

use reqwest::StatusCode;
use serde_json::{json, Value};
use thrust_contract_tests::*;
use thrust_contracts::{
    default_project_identifier, ProjectId, ProjectNetwork, Role, SeedFixture, SeedMembership, SeedProject,
    SeedProjectDetails, SeedState, SeedWorkspace, SeedWorkspaceRole, StateGroup, WorkspaceId, WorkspaceSlug,
};

use super::t;

pub(crate) const STRONG: &str = "Contract-Sample-Pa55word-Zebra-81";

pub(crate) fn new_email() -> String {
    format!("signup-{}@example.test", uuid::Uuid::new_v4().simple())
}

pub(crate) async fn post(t: &Target, path: &str, body: &Value) -> reqwest::Response {
    t.http.post(t.url(path)).json(body).send().await.unwrap()
}

/// Whether the response sets a live session cookie (a value, not a clearing), with the attributes that make it safe.
pub(crate) fn session_cookie(r: &reqwest::Response) -> Option<String> {
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
pub(crate) async fn signed_up(t: &Target) -> (String, String) {
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

    let r = patch(
        &t,
        &token,
        "/api/users/me/",
        &json!({"first_name": "  Journey ", "last_name": "Tester", "user_timezone": "Asia/Kolkata"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let long: Value = r.json().await.unwrap();
    assert_eq!(
        (long["first_name"].as_str(), long["last_name"].as_str(), long["user_timezone"].as_str()),
        (Some("Journey"), Some("Tester"), Some("Asia/Kolkata")),
        "{long}"
    );
    assert_eq!(
        long.as_object().unwrap().len(),
        39,
        "the long user has every column of the pin's User but the password"
    );
    assert!(long.get("avatar_url").is_none(), "the long user has no avatar_url (that is the short user's)");

    for (what, body, field, message) in [
        (
            "a URL in the first name",
            json!({"first_name": "see example.com"}),
            "first_name",
            "First name cannot contain a URL.",
        ),
        (
            "a URL in the last name",
            json!({"last_name": "http://example.test"}),
            "last_name",
            "Last name cannot contain a URL.",
        ),
        ("a blank display name", json!({"display_name": ""}), "display_name", "This field may not be blank."),
        (
            "an unknown zone",
            json!({"user_timezone": "Mars/Olympus"}),
            "user_timezone",
            "\"Mars/Olympus\" is not a valid choice.",
        ),
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
    assert_eq!(
        (after["first_name"].as_str(), after["user_timezone"].as_str()),
        (Some("Journey"), Some("Asia/Kolkata")),
        "and left what was there"
    );

    // the pin ignores the fields it marks read only, instead of refusing them
    let r = patch(
        &t,
        &token,
        "/api/users/me/",
        &json!({"email": "someone@else.test", "is_bot": true, "username": "chosen"}),
    )
    .await;
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
    assert_eq!(
        (p["is_onboarded"].as_bool(), p["language"].as_str(), p["start_of_the_week"].as_i64()),
        (Some(false), Some("en"), Some(0)),
        "{p}"
    );
    assert_eq!(
        p["onboarding_step"],
        json!({"profile_complete": false, "workspace_create": false, "workspace_invite": false, "workspace_join": false})
    );
    assert!(p["last_workspace_id"].is_null() && p["role"].is_null());

    let r = patch(&t, &token, "/api/users/me/profile/", &json!({"role": "developer"})).await;
    assert_eq!(r.status(), 200);
    let r = patch(
        &t,
        &token,
        "/api/users/me/profile/",
        &json!({"use_case": "Plan work", "start_of_the_week": "2", "has_marketing_email_consent": "yes"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let after = get(&t, &token, "/api/users/me/profile/").await;
    assert_eq!(
        (after["role"].as_str(), after["use_case"].as_str()),
        (Some("developer"), Some("Plan work")),
        "the second patch kept the first's role: {after}"
    );
    assert_eq!(
        (after["start_of_the_week"].as_i64(), after["has_marketing_email_consent"].as_bool()),
        (Some(2), Some(true)),
        "DRF's choice and boolean spellings"
    );

    for (what, body, field, message) in [
        (
            "a start of the week out of range",
            json!({"start_of_the_week": 9}),
            "start_of_the_week",
            "\"9\" is not a valid choice.",
        ),
        ("a boolean that is not one", json!({"is_onboarded": "maybe"}), "is_onboarded", "Must be a valid boolean."),
        (
            "a last workspace that is not an id",
            json!({"last_workspace_id": "nope"}),
            "last_workspace_id",
            "Must be a valid UUID.",
        ),
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
        assert_eq!(
            get(&t, &token, "/api/users/me/profile/").await[flag],
            true,
            "{path}: a refused body does not change it"
        );

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
        (
            "a taken email and a weak password (taken is checked first)",
            json!({"email": email, "password": "password"}),
            5030,
        ),
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

    for (what, email) in
        [("the seeded email", u.email.clone()), ("another spelling", format!(" {} ", u.email.to_uppercase()))]
    {
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
        (
            "a deactivated user with the right password",
            json!({"email": inactive.email, "password": inactive.password}),
            5065,
        ),
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
    assert_eq!(
        (settings["workspace"]["fallback_workspace_slug"].clone(), settings["workspace"]["invites"].clone()),
        (Value::Null, json!(0)),
        "{settings}"
    );
    assert!(slug_is_free(&t, &token, &slug).await);

    let r = post_as(
        &t,
        &token,
        "/api/workspaces/",
        &json!({"name": "Contract Workspace", "slug": slug, "organization_size": "2-10"}),
    )
    .await;
    assert_eq!(r.status(), 201);
    let made: Value = r.json().await.unwrap();
    assert_eq!(made.as_object().unwrap().len(), 17, "{made}");
    assert_eq!(
        (made["slug"].as_str(), made["name"].as_str(), made["role"].as_i64(), made["total_members"].as_i64()),
        (Some(slug.as_str()), Some("Contract Workspace"), Some(20), Some(1)),
        "{made}"
    );
    assert_eq!(
        (made["owner"].clone(), made["created_by"].clone(), made["organization_size"].clone()),
        (me["id"].clone(), me["id"].clone(), json!("2-10")),
        "{made}"
    );

    // theirs: listed, taken, the first one to open
    let listed = get(&t, &token, "/api/users/me/workspaces/").await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert_eq!((listed[0]["id"].clone(), listed[0]["role"].clone()), (made["id"].clone(), json!(20)));
    assert!(!slug_is_free(&t, &token, &slug).await, "a made workspace's slug is taken");
    let settings = get(&t, &token, "/api/users/me/settings/").await;
    assert_eq!(settings["workspace"]["fallback_workspace_slug"], json!(slug), "{settings}");
    assert_eq!(
        get(&t, &token, "/api/instances/").await["instance"]["workspaces_exist"],
        true,
        "S01 after the first workspace"
    );

    // what it says about its creator
    let member = get(&t, &token, &format!("/api/workspaces/{slug}/workspace-members/me/")).await;
    assert_eq!(
        (member["role"].as_i64(), member["is_active"].as_bool(), member["company_role"].as_str()),
        (Some(20), Some(true), Some("")),
        "{member}"
    );
    let members = get(&t, &token, &format!("/api/workspaces/{slug}/members/")).await;
    assert_eq!(members.as_array().unwrap().len(), 1, "{members}");
    assert_eq!(
        (members[0]["role"].clone(), members[0]["member"]["id"].clone(), members[0]["member"]["email"].clone()),
        (json!(20), me["id"].clone(), json!(email)),
        "{members}"
    );
    assert_eq!(
        get(&t, &token, &format!("/api/users/me/workspaces/{slug}/project-roles/")).await,
        json!({}),
        "a new workspace has no project"
    );

    // not theirs: nothing listed, nothing readable, nothing to take
    assert_eq!(get(&t, &other, "/api/users/me/workspaces/").await, json!([]));
    let r = t.http.get(t.url(&format!("/api/workspaces/{slug}/members/"))).bearer_auth(&other).send().await.unwrap();
    assert_eq!(r.status(), 403);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "You don't have the required permissions."}));
    let r = t
        .http
        .get(t.url(&format!("/api/users/me/workspaces/{slug}/project-roles/")))
        .bearer_auth(&other)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"detail": "You do not have permission to perform this action."})
    );
    let blank = get(&t, &other, &format!("/api/workspaces/{slug}/workspace-members/me/")).await;
    assert_eq!(
        (blank["is_active"].clone(), blank["member"].clone(), blank["role"].clone()),
        (json!(false), Value::Null, Value::Null),
        "{blank}"
    );
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
    let r =
        t.http.post(t.url("/api/workspaces/")).json(&json!({"name": "x", "slug": new_slug()})).send().await.unwrap();
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
        (
            "a name that is too long",
            json!({"name": "x".repeat(81), "slug": new_slug()}),
            400,
            json!({"error": "The maximum length for name is 80 and for slug is 48"}),
        ),
        (
            "a slug that is too long",
            json!({"name": "Long Slug", "slug": "s".repeat(49)}),
            400,
            json!({"error": "The maximum length for name is 80 and for slug is 48"}),
        ),
        (
            "a URL in the name",
            json!({"name": "see example.com", "slug": url_name}),
            400,
            json!({"error": "Name cannot contain a URL"}),
        ),
        (
            "a name with no letter or number",
            json!({"name": "!!!", "slug": no_alnum}),
            400,
            json!({"name": ["Name must contain at least one letter or number"]}),
        ),
        (
            "a reserved slug",
            json!({"name": "Reserved", "slug": "onboarding"}),
            400,
            json!({"slug": ["Slug is not valid"]}),
        ),
        (
            "a slug with a space",
            json!({"name": "Spaced", "slug": "a b"}),
            400,
            json!({"slug": ["Enter a valid \"slug\" consisting of letters, numbers, underscores or hyphens."]}),
        ),
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
async fn compat_opening_a_workspace_reads_the_sidebar_the_home_page_and_the_list_settings_once_and_refuses_everyone_else(
) {
    let t = t();
    let (_, token) = signed_up(&t).await;
    let (_, stranger) = signed_up(&t).await;
    let slug = new_slug();
    let made = post_as(&t, &token, "/api/workspaces/", &json!({"name": "Opening", "slug": slug})).await;
    assert_eq!(made.status(), 201);
    let made: Value = made.json().await.unwrap();
    let path = |tail: &str| format!("/api/workspaces/{slug}/{tail}");

    for tail in [
        "projects/",
        "user-favorites/",
        "states/",
        "quick-links/",
        "recent-visits/",
        "recent-visits/?entity_name=project",
    ] {
        assert_eq!(get(&t, &token, &path(tail)).await, json!([]), "{tail}");
    }
    assert_eq!(
        get(&t, &token, &path("users/notifications/unread/")).await,
        json!({"total_unread_notifications_count": 0, "mention_unread_notifications_count": 0})
    );

    let sidebar = get(&t, &token, &path("sidebar-preferences/")).await;
    let pins: Vec<(String, bool, f64)> = {
        let mut v: Vec<_> = sidebar
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, p)| (k.clone(), p["is_pinned"].as_bool().unwrap(), p["sort_order"].as_f64().unwrap()))
            .collect();
        v.sort_by(|a, b| a.2.total_cmp(&b.2));
        v
    };
    let want: Vec<(String, bool, f64)> = [
        ("views", false),
        ("active_cycles", false),
        ("analytics", false),
        ("drafts", true),
        ("your_work", true),
        ("archives", false),
        ("stickies", true),
    ]
    .iter()
    .enumerate()
    .map(|(i, (k, pinned))| ((*k).to_string(), *pinned, 65535.0 + 10000.0 * i as f64))
    .collect();
    assert_eq!(pins, want, "{sidebar}");

    let home = get(&t, &token, &path("home-preferences/")).await;
    let widgets: Vec<(&str, f64)> = home
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["key"].as_str().unwrap(), w["sort_order"].as_f64().unwrap()))
        .collect();
    assert_eq!(widgets, [("my_stickies", 997.0), ("recents", 998.0), ("quick_links", 999.0)], "{home}");

    let props = get(&t, &token, &path("user-properties/")).await;
    assert_eq!(
        (
            props["workspace"].clone(),
            props["navigation_project_limit"].clone(),
            props["navigation_control_preference"].clone()
        ),
        (made["id"].clone(), json!(10), json!("ACCORDION")),
        "{props}"
    );
    assert_eq!(props["display_filters"]["display_filters"]["layout"], "list", "{props}");

    // again: the same rows, not new ones
    assert_eq!(get(&t, &token, &path("sidebar-preferences/")).await, sidebar);
    assert_eq!(get(&t, &token, &path("home-preferences/")).await, home);
    assert_eq!(
        get(&t, &token, &path("user-properties/")).await["id"],
        props["id"],
        "the list settings were made again"
    );

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

/// S30, S32 and S34 to S46 for a person who has just made a workspace and a project in it. The project is theirs: they are its admin
/// and its only member, it has the pin's five states and an intake state, and every read the app makes when it opens one answers
/// 200 with what that is (the stubs, S42 to S46, with the empty list). Controls, each paired with the person it is not: a second
/// account is refused every one of those reads with the body the pin's guard for it gives, nobody without a session is let in, a
/// project that breaks a rule is refused with the pin's code and makes nothing, and a name or key that is taken is refused.
#[tokio::test]
async fn compat_a_project_is_made_by_its_creator_who_is_its_admin_and_the_app_reads_what_it_needs_of_it() {
    let t = t();
    let (_, token) = signed_up(&t).await;
    let (_, other) = signed_up(&t).await;
    let slug = new_slug();
    assert_eq!(post_as(&t, &token, "/api/workspaces/", &json!({"name": "Projects", "slug": slug})).await.status(), 201);
    let me = get(&t, &token, "/api/users/me/").await["id"].clone();
    let projects = format!("/api/workspaces/{slug}/projects/");

    let payload = json!({"name": "Journey Project", "identifier": "JRN", "network": 2, "description": "", "project_lead": null,
        "logo_props": {"in_use": "emoji", "emoji": {"value": "128512"}}, "cover_image_url": "/cover-images/image_5.webp"});
    let r = post_as(&t, &token, &projects, &payload).await;
    assert_eq!(r.status(), 201);
    let made: Value = r.json().await.unwrap();
    assert_eq!(made.as_object().unwrap().len(), 44, "{made}");
    assert_eq!(
        (
            made["member_role"].clone(),
            made["members"].clone(),
            made["sort_order"].clone(),
            made["next_work_item_sequence"].clone()
        ),
        (json!(20), json!([me]), json!(65535.0), json!(1)),
        "{made}"
    );
    let id = made["id"].as_str().unwrap().to_string();
    let p = |tail: &str| format!("{projects}{id}/{tail}");

    // what the app reads when it opens the project
    assert_eq!(get(&t, &token, &p("")).await["identifier"], "JRN");
    let row = get(&t, &token, &p("project-members/me/")).await;
    assert_eq!(
        (
            row["role"].clone(),
            row["member"]["id"].clone(),
            row["project"]["identifier"].clone(),
            row["workspace"]["slug"].clone()
        ),
        (json!(20), me.clone(), json!("JRN"), json!(slug)),
        "{row}"
    );
    let props = get(&t, &token, &p("user-properties/")).await;
    assert_eq!(
        (props["user"].clone(), props["sort_order"].clone(), props["display_filters"]["layout"].clone()),
        (me.clone(), json!(65535.0), json!("list")),
        "{props}"
    );
    assert_eq!(get(&t, &token, &p("user-properties/")).await["id"], props["id"], "asking again made another row");
    assert_eq!(get(&t, &token, &p("issue-labels/")).await, json!([]));
    let members = get(&t, &token, &p("members/")).await;
    assert_eq!(
        (members.as_array().unwrap().len(), members[0]["member"].clone(), members[0]["role"].clone()),
        (1, me.clone(), json!(20)),
        "{members}"
    );
    let states = get(&t, &token, &p("states/")).await;
    let names: Vec<&str> = states.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Backlog", "Todo", "In Progress", "Done", "Cancelled"], "{states}");
    assert_eq!(get(&t, &token, &format!("/api/workspaces/{slug}/states/")).await.as_array().unwrap().len(), 5);
    let triage = get(&t, &token, &p("intake-state/")).await;
    assert_eq!((triage["group"].clone(), triage["name"].clone()), (json!("triage"), json!("Triage")), "{triage}");
    for tail in ["estimates/", "cycles/", "modules/", "views/"] {
        assert_eq!(get(&t, &token, &p(tail)).await, json!([]), "{tail}");
    }
    assert_eq!(get(&t, &token, &format!("/api/workspaces/{slug}/modules/")).await, json!([]));
    let stats = get(&t, &token, &format!("/api/workspaces/{slug}/project-stats/")).await;
    assert_eq!(
        stats,
        json!([{"id": id, "total_issues": 0, "completed_issues": 0, "total_members": 1, "total_cycles": 0, "total_modules": 0}])
    );
    let listed = get(&t, &token, &projects).await;
    assert_eq!(
        (
            listed.as_array().unwrap().len(),
            listed[0]["member_role"].clone(),
            listed[0]["sort_order"].clone(),
            listed[0]["name"].clone()
        ),
        (1, json!(20), json!(65535.0), json!("Journey Project")),
        "{listed}"
    );

    // changing it: the app sends the cover image after the upload, which changes nothing but who and when
    let patch = |who: &str, body: Value| {
        let (t, url) = (&t, t.url(&p("")));
        let who = who.to_string();
        async move { t.http.patch(url).bearer_auth(who).json(&body).send().await.unwrap() }
    };
    let r = patch(&token, json!({"cover_image_url": "/cover-images/image_5.webp"})).await;
    assert_eq!(r.status(), 200);
    let changed: Value = r.json().await.unwrap();
    assert_eq!((changed["updated_by"].clone(), changed["name"].clone()), (me.clone(), json!("Journey Project")));
    let r = patch(&token, json!({"name": "Renamed", "description": "d"})).await;
    assert_eq!(r.json::<Value>().await.unwrap()["name"], "Renamed");

    // refusals make nothing, and say why in the pin's words
    for (what, body, want) in [
        (
            "a name that is taken",
            json!({"name": "Renamed", "identifier": "NEW"}),
            json!({"name": ["PROJECT_NAME_ALREADY_EXIST"]}),
        ),
        (
            "a key that is taken",
            json!({"name": "Another", "identifier": "JRN"}),
            json!({"identifier": ["PROJECT_IDENTIFIER_ALREADY_EXIST"]}),
        ),
        (
            "no fields",
            json!({}),
            json!({"name": ["This field is required."], "identifier": ["This field is required."]}),
        ),
        (
            "a forbidden character",
            json!({"name": "A.B", "identifier": "AB"}),
            json!({"name": ["PROJECT_NAME_CANNOT_CONTAIN_SPECIAL_CHARACTERS"]}),
        ),
        (
            "a network that is not one of the two",
            json!({"name": "Net", "identifier": "NET", "network": 3}),
            json!({"network": ["\"3\" is not a valid choice."]}),
        ),
    ] {
        let r = post_as(&t, &token, &projects, &body).await;
        assert_eq!(r.status(), 400, "{what}");
        assert_eq!(r.json::<Value>().await.unwrap(), want, "{what}");
    }
    assert_eq!(get(&t, &token, &projects).await.as_array().unwrap().len(), 1, "a refused project was made");

    // the second account: no member of the workspace, so each read is refused with its own guard's body, and nothing is changed
    for (tail, class) in [
        ("", false),
        ("user-properties/", false),
        ("members/", false),
        ("states/", false),
        ("intake-state/", false),
        ("estimates/", true),
        ("issue-labels/", true),
    ] {
        let r = t.http.get(t.url(&p(tail))).bearer_auth(&other).send().await.unwrap();
        assert_eq!(r.status(), 403, "a stranger on {tail}");
        let want = if class {
            json!({"detail": "You do not have permission to perform this action."})
        } else {
            json!({"error": "You don't have the required permissions."})
        };
        assert_eq!(r.json::<Value>().await.unwrap(), want, "a stranger on {tail}");
    }
    let r = t.http.get(t.url(&p("project-members/me/"))).bearer_auth(&other).send().await.unwrap();
    assert_eq!(
        (r.status().as_u16(), r.json::<Value>().await.unwrap()),
        (404, json!({"error": "The required object does not exist."}))
    );
    let r = patch(&other, json!({"name": "Mine now"})).await;
    assert_eq!(
        (r.status().as_u16(), r.json::<Value>().await.unwrap()),
        (403, json!({"error": "You don't have the required permissions."}))
    );
    assert_eq!(get(&t, &token, &p("")).await["name"], "Renamed", "a refused change changed the project");
    let r = t.http.get(t.url(&p(""))).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

/// S34 and the guards of S30 over a seeded workspace, whose members are members of it, one of them a guest of the workspace. A member
/// of the project opens it; a member of the workspace who is not in it is told it is public (409) or secret (403); a guest of the
/// workspace may open a project they are in but not make one, and a member may make one and is then its admin.
#[tokio::test]
async fn compat_a_project_is_opened_by_its_members_and_refused_to_the_rest_of_the_workspace_in_the_pins_ways() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let secret = ProjectId(uuid::Uuid::new_v4());
    let seeded = SeedFixture {
        workspaces: vec![SeedWorkspace { id: f.ws_a, slug: WorkspaceSlug::new(&f.a.slug) }],
        workspace_roles: vec![SeedWorkspaceRole { workspace_id: f.ws_a, user_id: f.guest_a.id, role: Role::Guest }],
        projects: vec![SeedProject { id: secret, workspace_id: f.ws_a, name: format!("Secret {}", secret.0.simple()) }],
        project_details: vec![SeedProjectDetails { network: ProjectNetwork::Secret, ..SeedProjectDetails::of(secret) }],
        memberships: vec![SeedMembership {
            project_id: secret,
            workspace_id: f.ws_a,
            user_id: f.member_a.id,
            role: Role::Member,
        }],
        ..SeedFixture::default()
    };
    assert_eq!(t.admin_post("/internal/seed", &serde_json::to_value(&seeded).unwrap()).await.status(), 204);
    let open = |who: &User, project: ProjectId| {
        let url = t.url(&format!("/api/workspaces/{}/projects/{}/", f.a.slug, project.0));
        let token = who.token.clone();
        let http = t.http.clone();
        async move {
            let r = http.get(url).bearer_auth(token).send().await.unwrap();
            (r.status().as_u16(), r.json::<Value>().await.unwrap())
        }
    };
    // a member of the project opens it (a guest of the workspace too), a member of the workspace who is not in it does not
    for who in [&f.admin_a, &f.member_a, &f.guest_a] {
        let (status, body) = open(who, f.a.project).await;
        assert_eq!((status, body["id"].clone()), (200, json!(f.a.project.0)), "{body}");
    }
    assert_eq!(open(&f.outsider_a, f.a.project).await, (409, json!({"error": "You are not a member of this project"})));
    assert_eq!(open(&f.outsider_a, secret).await, (403, json!({"error": "You do not have permission"})));
    assert_eq!(open(&f.member_a, secret).await.0, 200);
    assert_eq!(
        open(&f.admin_a, secret).await,
        (403, json!({"error": "You do not have permission"})),
        "an admin of the project A is not in the secret one"
    );
    assert_eq!(
        open(&f.member_a, ProjectId(uuid::Uuid::new_v4())).await,
        (404, json!({"error": "Project does not exist"}))
    );
    // the guest of the workspace may not make a project; a member may, and is its admin
    let projects = format!("/api/workspaces/{}/projects/", f.a.slug);
    let r = post_as(
        &t,
        &f.guest_a.token,
        &projects,
        &json!({"name": format!("G {}", secret.0.simple()), "identifier": "GST"}),
    )
    .await;
    assert_eq!(r.status(), 403);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "You don't have the required permissions."}));
    let r = post_as(&t, &f.member_a.token, &projects, &json!({"name": format!("M {}", secret.0.simple()), "identifier": format!("M{}", &secret.0.simple().to_string()[..8])})).await;
    assert_eq!(r.status(), 201);
    let made: Value = r.json().await.unwrap();
    assert_eq!((made["member_role"].clone(), made["members"].clone()), (json!(20), json!([f.member_a.id.0])), "{made}");
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
    let made: Value =
        post_as(&t, &admin, "/api/workspaces/", &json!({"name": "Listed", "slug": slug})).await.json().await.unwrap();
    let ws = WorkspaceId(made["id"].as_str().unwrap().parse().unwrap());
    let secret = ProjectId(uuid::Uuid::new_v4());
    let seeded = SeedFixture {
        workspaces: vec![SeedWorkspace { id: ws, slug: WorkspaceSlug::new(&slug) }],
        projects: vec![SeedProject { id: secret, workspace_id: ws, name: "Secret".into() }],
        project_details: vec![SeedProjectDetails {
            identifier: Some("SEC".into()),
            network: ProjectNetwork::Secret,
            ..SeedProjectDetails::of(secret)
        }],
        ..SeedFixture::default()
    };
    assert_eq!(t.admin_post("/internal/seed", &serde_json::to_value(&seeded).unwrap()).await.status(), 204);
    let listed = get(&t, &admin, &format!("/api/workspaces/{slug}/projects/")).await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    let p = &listed[0];
    assert_eq!(
        (p["id"].clone(), p["identifier"].clone(), p["network"].clone(), p["member_role"].clone()),
        (json!(secret.0), json!("SEC"), json!(0), Value::Null),
        "{p}"
    );
    assert_eq!(p.as_object().unwrap().len(), 21, "{p}");

    // the members: a seeded workspace whose members are all members of it, one project they are in and the secret one they are not
    let f = Fixture::seed(&t).await;
    let hidden = ProjectId(uuid::Uuid::new_v4());
    let seeded = SeedFixture {
        workspaces: vec![SeedWorkspace { id: f.ws_a, slug: WorkspaceSlug::new(&f.a.slug) }],
        projects: vec![SeedProject { id: hidden, workspace_id: f.ws_a, name: "Hidden".into() }],
        project_details: vec![SeedProjectDetails { network: ProjectNetwork::Secret, ..SeedProjectDetails::of(hidden) }],
        memberships: vec![SeedMembership {
            project_id: hidden,
            workspace_id: f.ws_a,
            user_id: f.admin_a.id,
            role: Role::Admin,
        }],
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
    assert_eq!(
        (outsider[0]["name"].clone(), outsider[0]["member_role"].clone()),
        (json!("A"), Value::Null),
        "{outsider}"
    );
}

/// S48, S47 and X01 on every target. A member of the project makes a work item (201, in the pin's 26 keys, numbered 1 and put at
/// 65535), the project lists it (the twelve keys of the pin's paginator, the 23 columns of a row) and the app opens it with what it
/// asks to be expanded; a guest of the project reads (sees only what they made) and may not make one; someone in the workspace who is
/// not in the project is the decorator's 403 on all three. Controls: the second work item is placed 10000 after the first, and the
/// stranger's refusals are on the same routes the members' successes are.
#[tokio::test]
async fn compat_a_member_makes_a_work_item_the_project_lists_it_and_the_app_opens_it() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let state = uuid::Uuid::new_v4();
    let seeded = SeedFixture {
        states: vec![SeedState {
            id: state,
            project_id: f.a.project,
            workspace_id: f.ws_a,
            name: "Backlog".into(),
            color: "#60646C".into(),
            sequence: 15000.0,
            group: StateGroup::Backlog,
            is_default: true,
        }],
        ..SeedFixture::default()
    };
    assert_eq!(t.admin_post("/internal/seed", &serde_json::to_value(&seeded).unwrap()).await.status(), 204);
    let issues = format!("/api/workspaces/{}/projects/{}/issues/", f.a.slug, f.a.project.0);
    let by_decorator = json!({"error": "You don't have the required permissions."});

    // a member makes one; a guest of the project and a member of the workspace who is not in it are refused
    let r = post_as(
        &t,
        &f.member_a.token,
        &issues,
        &json!({"name": "  The first  ", "priority": "high", "description_html": "<p>hi</p><script>x()</script>"}),
    )
    .await;
    assert_eq!(r.status(), 201);
    let made: Value = r.json().await.unwrap();
    assert_eq!(
        (made["name"].clone(), made["priority"].clone(), made["sequence_id"].clone(), made["sort_order"].clone()),
        (json!("The first"), json!("high"), json!(1), json!(65535.0)),
        "{made}"
    );
    assert_eq!(
        (made["state_id"].clone(), made["created_by"].clone()),
        (json!(state), json!(f.member_a.id.0)),
        "{made}"
    );
    assert_eq!(made.as_object().unwrap().len(), 26, "{made}");
    for who in [&f.guest_a, &f.outsider_a] {
        let r = post_as(&t, &who.token, &issues, &json!({"name": "no"})).await;
        assert_eq!((r.status().as_u16(), r.json::<Value>().await.unwrap()), (403, by_decorator.clone()));
    }
    let second: Value =
        post_as(&t, &f.admin_a.token, &issues, &json!({"name": "The second"})).await.json().await.unwrap();
    assert_eq!((second["sequence_id"].clone(), second["sort_order"].clone()), (json!(2), json!(75535.0)), "{second}");

    // the project lists them, newest first, in the pin's envelope; a member sees both, the guest only what they made (none)
    let first_page = "?cursor=100:0:0&per_page=100&order_by=-created_at&sub_issue=true&layout=list&filters=%7B%7D";
    let listed = get(&t, &f.admin_a.token, &format!("{issues}{first_page}")).await;
    assert_eq!(listed.as_object().unwrap().len(), 12, "{listed}");
    assert_eq!(
        (
            listed["count"].clone(),
            listed["total_count"].clone(),
            listed["next_cursor"].clone(),
            listed["prev_cursor"].clone()
        ),
        (json!(2), json!(2), json!("100:1:0"), json!("100:-1:1")),
        "{listed}"
    );
    let names: Vec<&str> = listed["results"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["The second", "The first"]);
    assert_eq!(
        listed["results"][0].as_object().unwrap().len(),
        26,
        "the 22 columns, the three arrays and the state's group: {}",
        listed["results"][0]
    );
    let guest = get(&t, &f.guest_a.token, &format!("{issues}{first_page}")).await;
    assert_eq!((guest["count"].clone(), guest["results"].clone()), (json!(0), json!([])), "{guest}");
    // a page of one, and what is refused
    let paged = get(&t, &f.admin_a.token, &format!("{issues}?per_page=1&cursor=1:1:0&order_by=sequence_id")).await;
    assert_eq!(
        (paged["results"][0]["name"].clone(), paged["next_page_results"].clone(), paged["prev_page_results"].clone()),
        (json!("The second"), json!(false), json!(true)),
        "{paged}"
    );
    let r =
        t.http.get(t.url(&format!("{issues}?group_by=priority"))).bearer_auth(&f.admin_a.token).send().await.unwrap();
    assert_eq!(r.status(), 400);
    let r = t.http.get(t.url(&format!("{issues}{first_page}"))).bearer_auth(&f.outsider_a.token).send().await.unwrap();
    assert_eq!((r.status().as_u16(), r.json::<Value>().await.unwrap()), (403, by_decorator.clone()));

    // the app opens the first, with what it expands; the guest may not open it, the stranger is refused, a work item that is not there is a 404
    let id = made["id"].as_str().unwrap();
    let open = format!("{issues}{id}/?expand=issue_reactions,issue_attachments,issue_link,parent");
    let opened = get(&t, &f.member_a.token, &open).await;
    assert_eq!(
        (
            opened["description_html"].clone(),
            opened["is_subscribed"].clone(),
            opened["issue_link"].clone(),
            opened["parent"].clone()
        ),
        (json!("<p>hi</p>"), json!(false), json!([]), Value::Null),
        "the script is gone: {opened}"
    );
    let r = t.http.get(t.url(&open)).bearer_auth(&f.guest_a.token).send().await.unwrap();
    assert_eq!(
        (r.status().as_u16(), r.json::<Value>().await.unwrap()),
        (403, json!({"error": "You are not allowed to view this issue"}))
    );
    let r = t.http.get(t.url(&open)).bearer_auth(&f.outsider_a.token).send().await.unwrap();
    assert_eq!((r.status().as_u16(), r.json::<Value>().await.unwrap()), (403, by_decorator));
    let r = t
        .http
        .get(t.url(&format!("{issues}{}/", uuid::Uuid::new_v4())))
        .bearer_auth(&f.member_a.token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        (r.status().as_u16(), r.json::<Value>().await.unwrap()),
        (404, json!({"error": "The required object does not exist."}))
    );
}

/// S47 and issue #51, on every target. The pin has no lower bound on a page size, so a size of 0 or less, and a cursor to the
/// previous page whose size is not the page size, end in its catch-all 500: status 500 and the body
/// `{"error":"Something went wrong please try again later"}` exactly, with nothing of why in it (the cause goes to the log). It is
/// a handled response and not a crash (ADR-0031, ADR-0006): the next request to the same target is answered, which on a Worker
/// (panic = abort) is the difference between a 500 and a lost request. All four are recorded on the pin (the S47 fixture).
/// Controls: a neighbouring size is a 200 on the same route; a size that is no number is the 400 and not the 500; a cursor that is
/// no cursor beats the size, as it does on the pin; and the permission check still comes before the size is read.
#[tokio::test]
async fn compat_a_page_size_of_zero_or_less_is_the_pins_500_and_the_target_keeps_answering() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let issues = format!("/api/workspaces/{}/projects/{}/issues/", f.a.slug, f.a.project.0);
    let planes_500 = r#"{"error":"Something went wrong please try again later"}"#;

    for query in ["per_page=0", "per_page=-1", "cursor=3:0:0&per_page=0", "cursor=5:1:1&per_page=3"] {
        let r = t.http.get(t.url(&format!("{issues}?{query}"))).bearer_auth(&f.admin_a.token).send().await.unwrap();
        assert_eq!(r.status(), 500, "{query}");
        let content_type = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
        assert!(content_type.starts_with("application/json"), "{query}: {content_type}");
        let body = r.text().await.unwrap();
        assert_eq!(body, planes_500, "{query}: the pin's body, byte for byte");
        assert!(!body.contains("per_page") && !body.contains("paginator"), "{query}: the cause is for the log: {body}");

        // the target is still there: the next request is answered, by the same route
        let next = get(&t, &f.admin_a.token, &format!("{issues}?per_page=1")).await;
        assert!(next["results"].is_array(), "{query}: {next}");
    }

    // controls: each of these is on the same route and is not the 500
    for (what, query, status) in [
        ("a neighbouring size", "per_page=1", 200),
        ("a size that is no number", "per_page=abc", 400),
        ("a size over the cap", "per_page=1001", 400),
        ("a cursor that is no cursor, with a size of zero", "cursor=abc&per_page=0", 400),
        ("a cursor to a page after the first, with a size below zero", "cursor=3:1:0&per_page=-1", 400),
        ("a cursor to the previous page, of the page size", "cursor=3:1:1&per_page=3", 200),
    ] {
        let r = t.http.get(t.url(&format!("{issues}?{query}"))).bearer_auth(&f.admin_a.token).send().await.unwrap();
        assert_eq!(r.status().as_u16(), status, "{what}: {query}");
        assert_ne!(r.text().await.unwrap(), planes_500, "{what}: {query}");
    }
    // the decorator answers before it reads a parameter: someone outside the project, and no one at all, are refused and not 500
    let r = t.http.get(t.url(&format!("{issues}?per_page=0"))).bearer_auth(&f.outsider_a.token).send().await.unwrap();
    assert_eq!(r.status(), 403);
    let r = t.http.get(t.url(&format!("{issues}?per_page=0"))).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

/// X03, X02 and what the work item's page asks besides, on every target. A member makes a work item with an assignee and changes it
/// (the name, the priority and the state, in one request): 204 and no body, and the work item is what was asked, completed because
/// the state is of the completed group. The history is written by the activity consumer, which every target runs apart from the
/// request (a task on a worker, a queue message), so it is read until it shows what was done: the creation, the assignee, then the
/// three changes in the order the request named them. A guest may not change another's work item, someone outside the project
/// may not change or read anything, a time that is no time is a 400 and the sub work items and relations are the empty answers.
/// Controls: the refusals are on the same routes as the successes, and the history read again by someone else is the same five rows.
#[tokio::test]
async fn compat_a_member_changes_a_work_item_and_its_history_shows_it() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let (backlog, done) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    let state = |id: uuid::Uuid, name: &str, sequence: f64, group: StateGroup, is_default: bool| SeedState {
        id,
        project_id: f.a.project,
        workspace_id: f.ws_a,
        name: name.into(),
        color: "#60646C".into(),
        sequence,
        group,
        is_default,
    };
    let seeded = SeedFixture {
        states: vec![
            state(backlog, "Backlog", 15000.0, StateGroup::Backlog, true),
            state(done, "Done", 30000.0, StateGroup::Completed, false),
        ],
        ..SeedFixture::default()
    };
    assert_eq!(t.admin_post("/internal/seed", &serde_json::to_value(&seeded).unwrap()).await.status(), 204);
    let issues = format!("/api/workspaces/{}/projects/{}/issues/", f.a.slug, f.a.project.0);
    let by_decorator = json!({"error": "You don't have the required permissions."});
    let by_class = json!({"detail": "You do not have permission to perform this action."});

    let made: Value =
        post_as(&t, &f.member_a.token, &issues, &json!({"name": "The first", "assignee_ids": [f.member_a.id.0]}))
            .await
            .json()
            .await
            .unwrap();
    let id = made["id"].as_str().unwrap().to_string();
    let item = format!("{issues}{id}/");
    let patch = |token: &str, path: String, body: Value| {
        let (http, url, token) = (t.http.clone(), t.url(&path), token.to_string());
        async move { http.patch(url).bearer_auth(token).json(&body).send().await.unwrap() }
    };

    // the change: 204 and no body, and the work item is as asked
    let r =
        patch(&f.member_a.token, item.clone(), json!({"name": "Renamed", "priority": "high", "state_id": done})).await;
    assert_eq!((r.status().as_u16(), r.bytes().await.unwrap().len()), (204, 0));
    let opened =
        get(&t, &f.member_a.token, &format!("{item}?expand=issue_reactions,issue_attachments,issue_link,parent")).await;
    assert_eq!(
        (opened["name"].clone(), opened["priority"].clone(), opened["state_id"].clone(), opened["updated_by"].clone()),
        (json!("Renamed"), json!("high"), json!(done), json!(f.member_a.id.0)),
        "{opened}"
    );
    assert!(opened["completed_at"].is_string(), "the state is of the completed group: {opened}");

    // the history, once the consumer has written it
    let history_path = format!("{item}history/?activity_type=issue-property");
    let rows = eventually(
        "the creation, the assignee and the three changes in the history",
        std::time::Duration::from_secs(20),
        || async {
            let rows = get(&t, &f.member_a.token, &history_path).await;
            (rows.as_array().unwrap().len() == 5).then_some(rows)
        },
    )
    .await;
    let fields: Vec<Value> = rows.as_array().unwrap().iter().map(|r| r["field"].clone()).collect();
    assert_eq!(
        fields,
        vec![Value::Null, json!("assignees"), json!("name"), json!("priority"), json!("state")],
        "{rows}"
    );
    assert_eq!(
        (rows[2]["old_value"].clone(), rows[2]["new_value"].clone(), rows[2]["comment"].clone()),
        (json!("The first"), json!("Renamed"), json!("updated the name to")),
        "{rows}"
    );
    assert_eq!(
        (rows[4]["old_value"].clone(), rows[4]["new_value"].clone(), rows[4]["new_identifier"].clone()),
        (json!("Backlog"), json!("Done"), json!(done)),
        "{rows}"
    );
    assert_eq!(rows[0]["issue_detail"]["name"], "Renamed", "the work item as it is now");
    // read again, by a guest (who may read any work item's history), it is the same
    let again = get(&t, &f.guest_a.token, &history_path).await;
    assert_eq!(again.as_array().unwrap().len(), 5, "{again}");

    // who may not: a guest changes no work item they did not make, and someone outside the project neither changes nor reads
    for (who, label) in [(&f.guest_a, "a guest"), (&f.outsider_a, "an outsider")] {
        let r = patch(&who.token, item.clone(), json!({"name": "no"})).await;
        assert_eq!((r.status().as_u16(), r.json::<Value>().await.unwrap()), (403, by_decorator.clone()), "{label}");
    }
    for tail in ["history/", "sub-issues/", "issue-relation/"] {
        let r = t.http.get(t.url(&format!("{item}{tail}"))).bearer_auth(&f.outsider_a.token).send().await.unwrap();
        assert_eq!((r.status().as_u16(), r.json::<Value>().await.unwrap()), (403, by_class.clone()), "{tail}");
    }
    assert_eq!(
        get(&t, &f.member_a.token, &format!("{issues}{id}/")).await["name"],
        "Renamed",
        "a refused change changed nothing"
    );
    let r = patch(&f.member_a.token, format!("{issues}{}/", uuid::Uuid::new_v4()), json!({})).await;
    assert_eq!((r.status().as_u16(), r.json::<Value>().await.unwrap()), (404, json!({"error": "Issue not found"})));
    let r = patch(&f.member_a.token, item.clone(), json!({"start_date": "yesterday"})).await;
    assert_eq!(
        (r.status().as_u16(), r.json::<Value>().await.unwrap()),
        (400, json!({"start_date": ["Date has wrong format. Use one of these formats instead: YYYY-MM-DD."]}))
    );

    // a time that is no time, a date far ahead, the comments, and the calls the page makes besides
    let r = t
        .http
        .get(t.url(&format!("{item}history/?created_at__gt=nonsense")))
        .bearer_auth(&f.member_a.token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        (r.status().as_u16(), r.json::<Value>().await.unwrap()),
        (400, json!({"error": "Please provide valid detail"}))
    );
    assert_eq!(
        get(&t, &f.member_a.token, &format!("{item}history/?activity_type=issue-property&created_at__gt=2099-01-01"))
            .await,
        json!([])
    );
    assert_eq!(get(&t, &f.member_a.token, &format!("{item}history/?activity_type=issue-comment")).await, json!([]));
    assert_eq!(
        get(&t, &f.guest_a.token, &format!("{item}sub-issues/")).await,
        json!({"sub_issues": [], "state_distribution": {}})
    );
    let relations = get(&t, &f.guest_a.token, &format!("{item}issue-relation/")).await;
    assert_eq!(relations.as_object().unwrap().len(), 8, "{relations}");
    assert!(relations.as_object().unwrap().values().all(|v| v == &json!([])), "{relations}");
}

// --- assets (issue #44, the asset stage: S28, S29, S31, S33) ----------------------------------------------------------------

async fn patch_as(t: &Target, token: &str, path: &str) -> reqwest::Response {
    t.http.patch(t.url(path)).bearer_auth(token).send().await.unwrap()
}

/// What S28 made: the asset's id and the address its bytes go to.
async fn made_asset(t: &Target, token: &str, slug: &str, picture: &[u8]) -> (String, String) {
    let body = json!({"name": "cover.png", "type": "image/png", "size": picture.len(), "entity_type": "PROJECT_COVER", "entity_identifier": ""});
    let r = post_as(t, token, &format!("/api/assets/v2/workspaces/{slug}/"), &body).await;
    assert_eq!(r.status(), 200);
    let made: Value = r.json().await.unwrap();
    let d = &made["upload_data"];
    assert_eq!((&d["method"], &d["fields"]), (&json!("PUT"), &json!({})), "{made}");
    assert_eq!(
        d["headers"],
        json!({"Content-Type": "image/png", "Content-Length": picture.len().to_string()}),
        "{made}"
    );
    assert_eq!(made["asset_url"], format!("/api/assets/v2/static/{}/", made["asset_id"].as_str().unwrap()), "{made}");
    (made["asset_id"].as_str().unwrap().to_string(), d["url"].as_str().unwrap().to_string())
}

/// What the browser does with the answer: the file to the address, with its type and no session at all.
async fn browser_put(t: &Target, url: &str, content_type: &str, bytes: &[u8]) -> reqwest::Response {
    t.http.put(t.url(url)).header("content-type", content_type).body(bytes.to_vec()).send().await.unwrap()
}

/// The stock app's project cover, S28 to S33, on every target: the upload goes to this API and lands in the target's blob store (R2 on
/// the Worker), is confirmed by looking at it, is attached to the project that was made after it, and is shown to anyone at the
/// address `cover_image_url` names. Each step has its refusal beside it: a second use of the address, a stranger's confirm, attach
/// and read, a read before the bytes are confirmed, and the upload of an asset that was never made.
#[tokio::test]
async fn compat_a_cover_is_uploaded_confirmed_attached_and_shown_and_each_step_refuses_what_is_not_its_own() {
    let t = t();
    let (_, token) = signed_up(&t).await;
    let (_, stranger) = signed_up(&t).await;
    let slug = new_slug();
    assert_eq!(post_as(&t, &token, "/api/workspaces/", &json!({"name": "Covers", "slug": slug})).await.status(), 201);
    let picture: Vec<u8> = (0u8..120).rev().collect();
    let assets = format!("/api/assets/v2/workspaces/{slug}/");

    // S28 and the browser's upload, with no session
    let (id, url) = made_asset(&t, &token, &slug, &picture).await;
    assert_eq!(
        patch_as(&t, &token, &format!("{assets}{id}/")).await.status(),
        400,
        "nothing is uploaded yet, and S29 looks"
    );
    assert_eq!(browser_put(&t, &url, "image/png", &picture).await.status(), 200);
    assert_eq!(browser_put(&t, &url, "image/png", &picture).await.status(), 403, "the address is good for one upload");

    // S29: a stranger to the workspace is refused, the maker confirms, and it is read for the workspace only after that
    let (pending, pending_url) = made_asset(&t, &token, &slug, &picture).await;
    assert_eq!(browser_put(&t, &pending_url, "image/png", &picture).await.status(), 200);
    let noredirect = t.no_redirect();
    let s33 = |token: &str, id: &str| {
        let req = noredirect.get(t.url(&format!("{assets}{id}/"))).bearer_auth(token.to_string());
        async move { req.send().await.unwrap() }
    };
    assert_eq!(s33(&token, &pending).await.status(), 404, "stored but not confirmed: nothing may read it");
    assert_eq!(patch_as(&t, &stranger, &format!("{assets}{id}/")).await.status(), 403);
    assert_eq!(patch_as(&t, &token, &format!("{assets}{id}/")).await.status(), 204);
    assert_eq!(patch_as(&t, &token, &format!("{assets}{id}/")).await.status(), 204, "confirming again changes nothing");

    // S33: a 302 to the route that sends the bytes, which asks who the caller is again
    let r = s33(&token, &id).await;
    assert_eq!(r.status(), 302);
    let location = r.headers()["location"].to_str().unwrap().to_string();
    assert_eq!(location, format!("{assets}{id}/object/"));
    assert_eq!(s33(&stranger, &id).await.status(), 403);
    let file = t.http.get(t.url(&location)).bearer_auth(&token).send().await.unwrap();
    assert_eq!(file.status(), 200);
    assert_eq!(file.headers()["content-type"], "image/png");
    assert_eq!(file.headers()["content-disposition"], "attachment; filename*=UTF-8''cover.png");
    assert_eq!(file.bytes().await.unwrap().as_ref(), picture.as_slice());
    assert_eq!(t.http.get(t.url(&location)).send().await.unwrap().status(), 401, "and not without a session");

    // S30 then S31: the project is made after the upload, and the cover reaches it by the bulk route
    let projects = format!("/api/workspaces/{slug}/projects/");
    let made: Value =
        post_as(&t, &token, &projects, &json!({"name": "Covered", "identifier": "COV"})).await.json().await.unwrap();
    let project = made["id"].as_str().unwrap().to_string();
    assert_eq!(made["cover_image_url"], Value::Null, "{made}");
    let bulk = format!("{assets}projects/{project}/{project}/bulk/");
    assert_eq!(post_as(&t, &stranger, &bulk, &json!({"asset_ids": [id]})).await.status(), 403);
    assert_eq!(post_as(&t, &token, &bulk, &json!({"asset_ids": []})).await.status(), 400);
    assert_eq!(
        post_as(&t, &token, &bulk, &json!({"asset_ids": [uuid::Uuid::new_v4().to_string()]})).await.status(),
        404
    );
    assert_eq!(post_as(&t, &token, &bulk, &json!({"asset_ids": [id]})).await.status(), 204);

    // S34, and the picture at the address it names: for anyone, inline, with its type pinned
    let opened = get(&t, &token, &format!("{projects}{project}/")).await;
    let shown = format!("/api/assets/v2/static/{id}/");
    assert_eq!(
        (opened["cover_image_url"].clone(), opened["cover_image_asset"].clone()),
        (json!(shown), json!(id)),
        "{opened}"
    );
    let r = t.http.get(t.url(&shown)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "image/png");
    assert_eq!(r.headers()["content-disposition"], "inline; filename*=UTF-8''cover.png");
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    assert_eq!(r.bytes().await.unwrap().as_ref(), picture.as_slice());
    // the project's asset is read through the workspace route by its people only
    assert_eq!(s33(&stranger, &id).await.status(), 403);
    // and an asset that was never confirmed is not shown to anyone
    assert_eq!(t.http.get(t.url(&format!("/api/assets/v2/static/{pending}/"))).send().await.unwrap().status(), 404);
}

/// The upload address is the credential of an upload and nothing else: it works for the file the asset was made for (its type and
/// exactly its size), once, and a body over the limit is refused before it is spent. Controls: an address of another asset, one
/// with a token that was never made, and an asset that does not exist all answer the same 403, and the right upload still works
/// after them.
#[tokio::test]
async fn compat_an_upload_is_refused_unless_it_is_the_file_the_asset_was_made_for() {
    let t = t();
    let (_, token) = signed_up(&t).await;
    let slug = new_slug();
    assert_eq!(post_as(&t, &token, "/api/workspaces/", &json!({"name": "Uploads", "slug": slug})).await.status(), 201);
    let picture = vec![7u8; 64];

    for (what, content_type, bytes) in [
        ("a type it was not made for", "image/gif", picture.clone()),
        ("a byte too many", "image/png", vec![7u8; 65]),
        ("a byte too few", "image/png", vec![7u8; 63]),
    ] {
        let (_, url) = made_asset(&t, &token, &slug, &picture).await;
        let r = browser_put(&t, &url, content_type, &bytes).await;
        assert_eq!(r.status(), 403, "{what}");
        assert_eq!(
            browser_put(&t, &url, "image/png", &picture).await.status(),
            403,
            "{what}: a refused upload spends the address"
        );
    }

    let (id, url) = made_asset(&t, &token, &slug, &picture).await;
    let (other_id, _) = made_asset(&t, &token, &slug, &picture).await;
    let token_of_it = url.trim_end_matches('/').rsplit('/').next().unwrap().to_string();
    for (what, address) in [
        ("another asset's", format!("/api/assets/v2/upload/{other_id}/{token_of_it}/")),
        ("a token that was never made", format!("/api/assets/v2/upload/{id}/{}/", "0".repeat(64))),
        ("an asset that does not exist", format!("/api/assets/v2/upload/{}/{token_of_it}/", uuid::Uuid::new_v4())),
    ] {
        assert_eq!(browser_put(&t, &address, "image/png", &picture).await.status(), 403, "{what}");
    }
    assert_eq!(browser_put(&t, "/api/assets/v2/upload/not-an-id/x/", "image/png", &picture).await.status(), 404);

    // one byte over the instance's 5 MiB is refused before the address is spent
    let limit = 5 * 1024 * 1024;
    let (_, big_url) = made_asset(&t, &token, &slug, &vec![0u8; limit]).await;
    assert_eq!(browser_put(&t, &big_url, "image/png", &vec![0u8; limit + 1]).await.status(), 413);
    assert_eq!(
        browser_put(&t, &big_url, "image/png", &vec![0u8; limit]).await.status(),
        200,
        "the address still works"
    );
    assert_eq!(
        browser_put(&t, &url, "image/png", &picture).await.status(),
        200,
        "and so does the first, after all that"
    );
}

// --- sign-out (issue #44, X06, D-036) ---------------------------------------------------------------------------------------

/// Both cookies a response sets, cleared: empty and expired at once.
fn assert_cookies_cleared(r: &reqwest::Response) {
    let cookies = SetCookie::all(r);
    for name in [SESSION_COOKIE, CSRF_COOKIE] {
        let c = SetCookie::find(&cookies, name).unwrap_or_else(|| panic!("{name} is not cleared: {cookies:?}"));
        assert!(c.value.is_empty() && c.has_attr("Max-Age=0"), "{c:?}");
    }
}

/// The stock app's sign-out, on every target: a JSON post that ends the session and answers where to go. A cookie mutation needs the
/// CSRF token wherever the deployment enforces it (the main pass does), and a sign-out that is refused ends nothing and clears
/// nothing: the session is read again with the same cookies to show it.
#[tokio::test]
async fn compat_sign_out_ends_the_session_clears_both_cookies_and_needs_the_csrf_token_for_a_cookie() {
    let t = t();
    let r = post(&t, "/auth/sign-up/", &json!({"email": new_email(), "password": STRONG})).await;
    assert_eq!(r.status(), 200);
    let cookies = SetCookie::all(&r);
    let session = SetCookie::find(&cookies, SESSION_COOKIE).expect("a session").value.clone();
    let csrf = SetCookie::find(&cookies, CSRF_COOKIE).expect("a csrf cookie").value.clone();
    let held = |req: reqwest::RequestBuilder| {
        req.header("cookie", format!("{SESSION_COOKIE}={session}; {CSRF_COOKIE}={csrf}"))
    };
    let me = || async { held(t.http.get(t.url("/api/users/me/"))).send().await.unwrap().status() };
    assert_eq!(me().await, 200);

    // refused: the cookie without the token, and with another one
    for (what, token) in [("no token", None), ("another token", Some("not-the-token"))] {
        let mut req = held(t.http.post(t.url("/auth/sign-out/")));
        if let Some(tok) = token {
            req = req.header(CSRF_HEADER, tok);
        }
        let r = req.send().await.unwrap();
        assert_eq!(r.status(), 403, "{what}");
        assert!(SetCookie::all(&r).is_empty(), "{what}: a refusal clears nothing");
        assert_eq!(me().await, 200, "{what}: and ends nothing");
    }

    // with the token the cookie holds, as the app sends it: ended, cleared, and told where to go
    let r = held(t.http.post(t.url("/auth/sign-out/")).header(CSRF_HEADER, &csrf)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_cookies_cleared(&r);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"redirect_path": "/"}));
    assert_eq!(me().await, 401, "the session is over");

    // and the same again for a session that has ended: the answer does not say what there was to end
    let r = held(t.http.post(t.url("/auth/sign-out/")).header(CSRF_HEADER, &csrf)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_cookies_cleared(&r);
}

/// What else answers a sign-out, and what ends: a request with no credential gets the same answer as any other (the pin redirects
/// whatever happens); a session held as a bearer token is ended (no CSRF token: a script's credential is not ambient); and an API
/// key ends nothing, because a leaked key must not be able to end the user's sessions.
#[tokio::test]
async fn compat_sign_out_answers_one_way_ends_a_bearer_session_and_is_not_done_by_an_api_key() {
    let t = t();
    let r = post(&t, "/auth/sign-out/", &json!({})).await;
    assert_eq!(r.status(), 200, "no credential at all");
    assert_cookies_cleared(&r);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"redirect_path": "/"}));

    let (_, token) = signed_up(&t).await;
    let key: thrust_contracts::CreatedApiKey = {
        let r = t
            .http
            .post(t.url(&format!("{SLICE_PREFIX}/api-keys")))
            .bearer_auth(&token)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
        r.json().await.unwrap()
    };
    let me = |req: reqwest::RequestBuilder| async { req.send().await.unwrap().status() };
    assert_eq!(me(t.http.get(t.url("/api/users/me/")).bearer_auth(&token)).await, 200);

    let r = t.http.post(t.url("/auth/sign-out/")).header("x-api-key", &key.token).send().await.unwrap();
    assert_eq!(r.status(), 200, "the answer is the same");
    assert_eq!(me(t.http.get(t.url("/api/users/me/")).bearer_auth(&token)).await, 200, "but a key ends no session");

    let r = t.http.post(t.url("/auth/sign-out/")).bearer_auth(&token).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_cookies_cleared(&r);
    assert_eq!(me(t.http.get(t.url("/api/users/me/")).bearer_auth(&token)).await, 401, "a bearer session is ended");
}

// --- change password (X09, D-039) -------------------------------------------------------------------------------------------

const NEW_STRONG: &str = "Contract-Another-Pa55word-Yak-27";

fn change_body(old: Option<&str>, new: Option<&str>) -> Value {
    let mut body = serde_json::Map::new();
    if let Some(old) = old {
        body.insert("old_password".into(), json!(old));
    }
    if let Some(new) = new {
        body.insert("new_password".into(), json!(new));
    }
    Value::Object(body)
}

async fn sign_in_status(t: &Target, email: &str, password: &str) -> StatusCode {
    post(t, "/auth/sign-in/", &json!({"email": email, "password": password})).await.status()
}

/// What Profile -> Security does, on every target: a JSON post of the old and the new password from a signed-in person. Refusals
/// come in Plane's order with its codes and change nothing (not a password, not a session, no cookie); a change is the new
/// password, every session of the account ended (the one that asked and another device's too) and one new session in the cookies,
/// so the person stays in. A cookie mutation needs the CSRF token wherever the deployment enforces it (the main pass does).
#[tokio::test]
async fn compat_change_password_refuses_in_planes_order_changes_nothing_then_swaps_it_and_ends_every_session() {
    let t = t();
    let email = new_email();
    let r = post(&t, "/auth/sign-up/", &json!({"email": email, "password": STRONG})).await;
    assert_eq!(r.status(), 200);
    let cookies = SetCookie::all(&r);
    let session = SetCookie::find(&cookies, SESSION_COOKIE).expect("a session").value.clone();
    let csrf = SetCookie::find(&cookies, CSRF_COOKIE).expect("a csrf cookie").value.clone();
    // another device: a second session of the same account
    let r = post(&t, "/auth/sign-in/", &json!({"email": email, "password": STRONG})).await;
    assert_eq!(r.status(), 200);
    let other = session_cookie(&r).expect("a second session");
    assert_ne!(other, session);

    let me = |token: &str| {
        let req = t.http.get(t.url("/api/users/me/")).bearer_auth(token);
        async move { req.send().await.unwrap().status() }
    };
    assert_eq!(
        (me(&session).await, me(&other).await),
        (StatusCode::OK, StatusCode::OK),
        "control: both sessions are live"
    );
    let change = |body: Value, token: Option<&str>| {
        let mut req = t
            .http
            .post(t.url("/auth/change-password/"))
            .header("cookie", format!("{SESSION_COOKIE}={session}; {CSRF_COOKIE}={csrf}"))
            .json(&body);
        if let Some(tok) = token {
            req = req.header(CSRF_HEADER, tok);
        }
        async move { req.send().await.unwrap() }
    };

    // refused for want of the token, as every cookie mutation is: nothing is looked at, so a right body changes nothing either
    for (what, tok) in [("no token", None), ("another token", Some("not-the-token"))] {
        let r = change(change_body(Some(STRONG), Some(NEW_STRONG)), tok).await;
        assert_eq!(r.status(), 403, "{what}");
        assert!(SetCookie::all(&r).is_empty(), "{what}");
    }

    // refused for the body, in the order of the pin, and each one says so in its code
    let missing = |text: &str| json!({"error_code": 5138, "error_message": "MISSING_PASSWORD", "error": text});
    let wrong =
        json!({"error_code": 5135, "error_message": "INCORRECT_OLD_PASSWORD", "error": "Old password is not correct"});
    let weak = json!({"error_code": 5021, "error_message": "PASSWORD_TOO_WEAK"});
    for (what, body, expected) in [
        ("no old password", change_body(None, Some(NEW_STRONG)), missing("Old password is missing")),
        ("no new password", change_body(Some(STRONG), None), missing("Old or new password is missing")),
        ("no old password and a weak new one", change_body(None, Some("password")), missing("Old password is missing")),
        ("a wrong old password", change_body(Some("not the password"), Some(NEW_STRONG)), wrong.clone()),
        ("a wrong old password and a weak new one", change_body(Some("not the password"), Some("password")), wrong),
        ("a weak new password", change_body(Some(STRONG), Some("password")), weak),
    ] {
        let r = change(body, Some(&csrf)).await;
        assert_eq!(r.status(), 400, "{what}");
        assert!(SetCookie::all(&r).is_empty(), "{what}: a refusal sets no cookie");
        assert_eq!(r.json::<Value>().await.unwrap(), expected, "{what}");
    }
    assert_eq!((me(&session).await, me(&other).await), (StatusCode::OK, StatusCode::OK), "a refusal ends no session");
    assert_eq!(sign_in_status(&t, &email, STRONG).await, 200, "and changes no password");
    assert_eq!(sign_in_status(&t, &email, NEW_STRONG).await, 400);

    // the change: answered as the pin answers, and the caller holds a session of the account in the cookies it sets
    let r = change(change_body(Some(STRONG), Some(NEW_STRONG)), Some(&csrf)).await;
    assert_eq!(r.status(), 200);
    let made = session_cookie(&r).expect("a change sets the caller's new session");
    assert!(
        SetCookie::find(&SetCookie::all(&r), CSRF_COOKIE).is_some_and(|c| !c.value.is_empty()),
        "and its CSRF cookie"
    );
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"message": "Password updated successfully"}));

    assert_eq!(me(&made).await, 200, "the caller stays signed in, on the session the change made");
    assert_eq!(me(&session).await, 401, "the session that asked is over");
    assert_eq!(me(&other).await, 401, "and so is every other session of the account");
    assert_eq!(sign_in_status(&t, &email, STRONG).await, 400, "the old password no longer signs in");
    assert_eq!(sign_in_status(&t, &email, NEW_STRONG).await, 200, "the new one does");
}

/// Who may do it: a session, and nothing else. A request with no credential or an unknown one is a 401; an API key is a script's
/// credential, and a leaked one must not take the account over, so it is a 401 too and changes nothing. Control: the same body
/// with the account's own session (a bearer token, which needs no CSRF token) is a change, so the 401s are not the route's constant
/// answer.
#[tokio::test]
async fn compat_change_password_is_for_a_session_and_not_for_an_api_key() {
    let t = t();
    let (email, token) = signed_up(&t).await;
    let key: thrust_contracts::CreatedApiKey = {
        let r = t
            .http
            .post(t.url(&format!("{SLICE_PREFIX}/api-keys")))
            .bearer_auth(&token)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
        r.json().await.unwrap()
    };
    let body = change_body(Some(STRONG), Some(NEW_STRONG));
    let send = |req: reqwest::RequestBuilder| async { req.json(&body).send().await.unwrap() };
    for (what, req) in [
        ("no credential", t.http.post(t.url("/auth/change-password/"))),
        ("an unknown bearer token", t.http.post(t.url("/auth/change-password/")).bearer_auth("not-a-session-token")),
        ("an API key", t.http.post(t.url("/auth/change-password/")).header("x-api-key", &key.token)),
        ("an API key as a bearer token", t.http.post(t.url("/auth/change-password/")).bearer_auth(&key.token)),
    ] {
        let r = send(req).await;
        assert_eq!(r.status(), 401, "{what}");
        assert!(SetCookie::all(&r).is_empty(), "{what}");
    }
    assert_eq!(sign_in_status(&t, &email, STRONG).await, 200, "nothing was changed");
    assert_eq!(sign_in_status(&t, &email, NEW_STRONG).await, 400);

    let r = send(t.http.post(t.url("/auth/change-password/")).bearer_auth(&token)).await;
    assert_eq!(r.status(), 200, "control: the account's own session changes it");
    assert_eq!(sign_in_status(&t, &email, NEW_STRONG).await, 200);
    assert_eq!(sign_in_status(&t, &email, STRONG).await, 400);
}

// ---------------------------------------------------------------------------------------------------------------------------------
// X10 (D-040): OIDC through the routes the stock app's "Sign in with SSO" button uses, `GET /auth/oidc/` and `GET /auth/oidc/callback/`,
// against the mock identity provider the suite serves on 127.0.0.1 (as the slice's own `oidc_` tests do, in `auth`). The mock registered one
// redirect URI, and the target does not look at the path a callback arrives on, so the same code and state are sent to the app's callback.
// A deployed Worker cannot reach 127.0.0.1: these `oidc_` tests are skipped by scripts/stage2-contract.sh.
// ---------------------------------------------------------------------------------------------------------------------------------

use thrust_contract_tests::mock_idp::{Callback, Identity, MockIdp, Tamper};

fn oidc_sub() -> String {
    format!("sub-{}", uuid::Uuid::new_v4().simple())
}

/// `GET /auth/oidc/` as the button does: a 302 to the provider, never cached. The app appends `?next_path=` when it has a place to go back to.
async fn compat_oidc_start(t: &Target, query: &str) -> String {
    let r = t.no_redirect().get(t.url(&format!("/auth/oidc/{query}"))).send().await.unwrap();
    assert_eq!(r.status(), 302, "start answers a redirect");
    assert_eq!(r.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-store"));
    assert!(SetCookie::all(&r).is_empty(), "a start sets no cookie");
    r.headers().get("location").and_then(|v| v.to_str().ok()).expect("a Location").to_string()
}

async fn compat_oidc_begin(t: &Target, identity: Identity, tamper: Tamper) -> Callback {
    // the provider first: the target discovers it while it handles the start
    let idp = MockIdp::shared();
    let location = compat_oidc_start(t, "").await;
    idp.authorize(&location, identity, tamper)
}

async fn compat_oidc_finish(t: &Target, cb: &Callback) -> reqwest::Response {
    t.no_redirect()
        .get(format!("{}?code={}&state={}", t.url("/auth/oidc/callback/"), cb.code, cb.state))
        .send()
        .await
        .unwrap()
}

/// A callback that was refused: a 302 to the app's root with the code the app reads (the pin's, PQ-213) and no cookie.
fn compat_oidc_refused(r: &reqwest::Response, code: u16, name: &str, what: &str) {
    assert_eq!(r.status(), 302, "{what}");
    let want = format!("/?error_code={code}&error_message={name}");
    assert_eq!(r.headers().get("location").and_then(|v| v.to_str().ok()), Some(want.as_str()), "{what}");
    assert!(SetCookie::all(r).is_empty(), "{what}: a refused login must not set a cookie");
}

/// The login the button starts completes, and ends in a session the rest of the app accepts: a 302 to the root with the session and CSRF cookies. Controls: the
/// start does not read `?next_path=` (a caller's place is not the provider's URL), and the first login of an account links the provider's subject, so the
/// subject decides afterwards (another account's verified email on the same subject is still the first account).
#[tokio::test]
async fn compat_oidc_login_completes_through_the_apps_routes_and_is_a_working_session() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let sub = oidc_sub();
    // the place the app asked to come back to is not in the provider's URL
    let with_place = compat_oidc_start(&t, "?next_path=/some-workspace/").await;
    assert!(!with_place.contains("some-workspace"), "{with_place}");

    let cb = compat_oidc_begin(&t, Identity::verified(&sub, &f.member_a.email), Tamper::None).await;
    let r = compat_oidc_finish(&t, &cb).await;
    assert_eq!(r.status(), 302);
    assert_eq!(r.headers().get("location").and_then(|v| v.to_str().ok()), Some("/"));
    assert_eq!(r.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-store"));
    let token = session_cookie(&r).expect("a session cookie");
    assert!(SetCookie::find(&SetCookie::all(&r), CSRF_COOKIE).is_some(), "a csrf cookie");
    let me = t.http.get(t.url("/api/users/me/")).bearer_auth(&token).send().await.unwrap();
    assert_eq!(me.status(), 200, "the cookie the callback set is a session");
    assert_eq!(me.json::<Value>().await.unwrap()["email"], f.member_a.email);

    // the link decides from now on: the same subject reporting another account's verified email is still the first account
    let cb = compat_oidc_begin(&t, Identity::verified(&sub, &f.admin_a.email), Tamper::None).await;
    let r = compat_oidc_finish(&t, &cb).await;
    let token = session_cookie(&r).expect("a session cookie");
    let me = t.http.get(t.url("/api/users/me/")).bearer_auth(&token).send().await.unwrap();
    assert_eq!(
        me.json::<Value>().await.unwrap()["email"],
        f.member_a.email,
        "the subject is linked to the first account"
    );
}

/// Every way a login can be refused is a 302 to the root with the code the app reads, and no cookie: `SIGNUP_DISABLED` (5015) for an identity Thrust will not let in
/// (no verified email, no account for it, a deactivated one: Thrust makes no account at a login, Q-0050.1), `OIDC_OAUTH_PROVIDER_ERROR` (5301) for a token that does not
/// verify and for a state or code that is missing, unknown, used or refused by the person. Control: none of the refusals that did not use the login's state up
/// stopped a genuine login from completing.
#[tokio::test]
async fn compat_oidc_every_refusal_is_a_redirect_with_the_code_the_app_reads() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let email = f.member_a.email.clone();
    let stranger = format!("stranger-{}@example.test", uuid::Uuid::new_v4().simple());

    // the identity is not let in: 5015
    for (what, identity) in [
        ("email_verified false", Identity { sub: oidc_sub(), email: Some(email.clone()), email_verified: Some(false) }),
        ("email_verified absent", Identity { sub: oidc_sub(), email: Some(email.clone()), email_verified: None }),
        ("no email", Identity { sub: oidc_sub(), email: None, email_verified: Some(true) }),
        ("a verified email nobody has", Identity::verified(&oidc_sub(), &stranger)),
    ] {
        let cb = compat_oidc_begin(&t, identity, Tamper::None).await;
        compat_oidc_refused(&compat_oidc_finish(&t, &cb).await, 5015, "SIGNUP_DISABLED", what);
    }
    // and no account was made for the stranger
    assert_eq!(sign_in_status(&t, &stranger, "anything").await, 400, "no user was made");
    // a deactivated user, by email and then by link
    let sub = oidc_sub();
    t.set_user_active(f.guest_a.id, false).await;
    let cb = compat_oidc_begin(&t, Identity::verified(&sub, &f.guest_a.email), Tamper::None).await;
    compat_oidc_refused(&compat_oidc_finish(&t, &cb).await, 5015, "SIGNUP_DISABLED", "a deactivated user, by email");
    t.set_user_active(f.guest_a.id, true).await;
    let cb = compat_oidc_begin(&t, Identity::verified(&sub, &f.guest_a.email), Tamper::None).await;
    assert!(
        session_cookie(&compat_oidc_finish(&t, &cb).await).is_some(),
        "control: reactivated, the same login is accepted"
    );
    t.set_user_active(f.guest_a.id, false).await;
    let cb = compat_oidc_begin(&t, Identity::verified(&sub, &f.guest_a.email), Tamper::None).await;
    compat_oidc_refused(&compat_oidc_finish(&t, &cb).await, 5015, "SIGNUP_DISABLED", "a deactivated user, by link");
    t.set_user_active(f.guest_a.id, true).await;

    // the provider's answer does not authenticate: 5301
    for tamper in
        [Tamper::WrongNonce, Tamper::WrongAudience, Tamper::WrongIssuer, Tamper::Expired, Tamper::BadSignature]
    {
        let cb = compat_oidc_begin(&t, Identity::verified(&oidc_sub(), &email), tamper).await;
        compat_oidc_refused(
            &compat_oidc_finish(&t, &cb).await,
            5301,
            "OIDC_OAUTH_PROVIDER_ERROR",
            &format!("{tamper:?}"),
        );
    }

    // the state or the code is not good: 5301, and these use nothing up
    let cb = compat_oidc_begin(&t, Identity::verified(&oidc_sub(), &email), Tamper::None).await;
    let flipped = if cb.state.ends_with('0') { '1' } else { '0' };
    let tampered = Callback { state: format!("{}{flipped}", &cb.state[..63]), ..cb.clone() };
    compat_oidc_refused(
        &compat_oidc_finish(&t, &tampered).await,
        5301,
        "OIDC_OAUTH_PROVIDER_ERROR",
        "a state with one character changed",
    );
    let unknown = Callback { state: "f".repeat(64), ..cb.clone() };
    compat_oidc_refused(
        &compat_oidc_finish(&t, &unknown).await,
        5301,
        "OIDC_OAUTH_PROVIDER_ERROR",
        "a state nobody issued",
    );
    let none = t.no_redirect().get(t.url("/auth/oidc/callback/")).send().await.unwrap();
    compat_oidc_refused(&none, 5301, "OIDC_OAUTH_PROVIDER_ERROR", "no code and no state");
    let denied = t
        .no_redirect()
        .get(format!("{}?error=access_denied&state={}", t.url("/auth/oidc/callback/"), cb.state))
        .send()
        .await
        .unwrap();
    compat_oidc_refused(&denied, 5301, "OIDC_OAUTH_PROVIDER_ERROR", "the provider reports an error");
    // control: none of that used the login up; the genuine callback completes, and then the same callback again finds nothing
    assert!(session_cookie(&compat_oidc_finish(&t, &cb).await).is_some(), "the genuine callback completes");
    compat_oidc_refused(
        &compat_oidc_finish(&t, &cb).await,
        5301,
        "OIDC_OAUTH_PROVIDER_ERROR",
        "the same callback again",
    );
}

/// The app draws the SSO button from `/api/instances/`: these targets have a provider configured, so it says so, and says no other method beyond the password is
/// on (the sign-in journey holds that on a browser). A deployment with none says `false`: that half is the hermetic tests' and the e2e pass's, which runs with no provider.
#[tokio::test]
async fn compat_oidc_the_instance_offers_sso_when_the_target_has_a_provider() {
    let t = t();
    let body: Value = t.http.get(t.url("/api/instances/")).send().await.unwrap().json().await.unwrap();
    let config = &body["config"];
    assert_eq!(config["is_oidc_enabled"], true, "{body}");
    assert_eq!(config["is_email_password_enabled"], true, "{body}");
    for off in [
        "is_google_enabled",
        "is_github_enabled",
        "is_gitlab_enabled",
        "is_gitea_enabled",
        "is_magic_login_enabled",
        "is_smtp_configured",
    ] {
        assert_eq!(config[off], false, "{off}: {body}");
    }
}
