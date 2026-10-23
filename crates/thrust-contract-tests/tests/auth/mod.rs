//! Identity over HTTP (issue #38; ADR-0050, ADR-0052): sign-in with an argon2id password, cookie sessions with CSRF
//! double-submit, sign-out and sign-out-everywhere, deactivation. The same assertions run on every target.
//!
//! Every test that asserts "X is accepted" is paired with a control asserting "the neighbouring Y is refused", and
//! `scripts/mutants.sh` runs the suite against a build that skips the CSRF check and requires it to fail.

use reqwest::Method;
use serde_json::{json, Value};
use thrust_contract_tests::*;
use thrust_contracts::*;

use super::t;

fn auth_url(t: &Target, tail: &str) -> String {
    t.url(&format!("{SLICE_PREFIX}/auth{tail}"))
}

/// A session's cookies sent as a browser would send them, with the CSRF token in its header unless `csrf` says otherwise.
fn as_browser(t: &Target, m: Method, url: String, s: &Session, csrf: Csrf<'_>) -> reqwest::RequestBuilder {
    let (cookie, header) = match csrf {
        Csrf::Matching => (s.cookie_header(), Some(s.csrf.clone())),
        Csrf::Absent => (s.cookie_header(), None),
        Csrf::Wrong(h) => (s.cookie_header(), Some(h.to_string())),
        Csrf::NoCookie => (s.session_cookie_only(), Some(s.csrf.clone())),
    };
    let r = t.http.request(m, url).header("cookie", cookie);
    match header {
        Some(h) => r.header(CSRF_HEADER, h),
        None => r,
    }
}

#[derive(Clone, Copy)]
enum Csrf<'a> {
    /// The header carries the token the cookie holds.
    Matching,
    /// A cookie, and no header.
    Absent,
    /// A header that is not the cookie's token.
    Wrong(&'a str),
    /// A header, and the session cookie without the CSRF cookie.
    NoCookie,
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

async fn error_code(r: reqwest::Response) -> String {
    r.json::<Value>().await.unwrap()["error"]["code"].as_str().unwrap_or_default().to_string()
}

/// Sign-in answers 200 with two cookies carrying the attributes ADR-0052 asks for, and the session behind the cookie
/// works; a wrong password and an unknown email get the same 401 with no cookie. (Spike 0003 step 2: the suite asserts
/// the attributes on `Set-Cookie` itself, because the local targets serve plain HTTP.)
#[tokio::test]
async fn sign_in_sets_http_only_cookies_and_the_session_works() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let r = t.sign_in_raw(&f.member_a.email, &f.member_a.password).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-store"));
    let cookies = SetCookie::all(&r);
    assert_eq!(cookies.len(), 2, "exactly the session and the CSRF cookie: {cookies:?}");
    let session = SetCookie::find(&cookies, SESSION_COOKIE).expect("session cookie");
    let csrf = SetCookie::find(&cookies, CSRF_COOKIE).expect("csrf cookie");
    assert!(is_hex(&session.value, 64), "256 random bits, hex: {}", session.value);
    assert!(is_hex(&csrf.value, 32), "128 random bits, hex: {}", csrf.value);
    for c in [session, csrf] {
        assert!(c.has_attr("Secure"), "{}: Secure", c.name);
        assert!(c.has_attr("HttpOnly"), "{}: HttpOnly", c.name);
        assert_eq!(c.attr("SameSite"), Some("Lax"), "{}", c.name);
        // the __Host- prefix needs Path=/ and forbids Domain
        assert_eq!(c.attr("Path"), Some("/"), "{}", c.name);
        assert_eq!(c.attr("Domain"), None, "{}", c.name);
        assert_eq!(c.attr("Max-Age"), Some("2592000"), "{}: 30 days", c.name);
        assert!(c.name.starts_with("__Host-"), "{}", c.name);
    }
    let text = r.text().await.unwrap();
    assert!(!text.contains(&session.value), "the session token must not be in the body");
    let body: SignInResponse = serde_json::from_str(&text).unwrap();
    assert_eq!((body.user_id, body.csrf_token.as_str()), (f.member_a.id, csrf.value.as_str()));

    // the cookie is a session: it authenticates a safe request, and so does its value as a bearer token
    let s = Session { user_id: body.user_id, token: session.value.clone(), csrf: csrf.value.clone() };
    let cookie_get = as_browser(&t, Method::GET, t.url(&wi_path(&f.a)), &s, Csrf::Absent).send().await.unwrap();
    assert_eq!(cookie_get.status(), 200, "a safe request needs no CSRF token");
    assert_eq!(list(&t, &User { token: s.token.clone(), ..clone_user(&f.member_a) }, &f.a, None).await.status(), 200);
    let me = as_browser(&t, Method::GET, auth_url(&t, "/session"), &s, Csrf::Absent).send().await.unwrap();
    assert_eq!(me.status(), 200);
    let me: SessionInfo = me.json().await.unwrap();
    assert_eq!(me, SessionInfo { user_id: f.member_a.id, credential: CredentialKind::Session });

    // controls: a wrong password and an unknown email are the same refusal, and set no cookie
    let wrong = t.sign_in_raw(&f.member_a.email, "not the password").await;
    let unknown = t.sign_in_raw("nobody@example.test", &f.member_a.password).await;
    assert_eq!((wrong.status().as_u16(), unknown.status().as_u16()), (401, 401));
    assert!(SetCookie::all(&wrong).is_empty() && SetCookie::all(&unknown).is_empty());
    let (wrong, unknown) = (wrong.text().await.unwrap(), unknown.text().await.unwrap());
    assert_eq!(wrong, unknown, "nothing in the answer says which one it was");
    // control: the email is matched case-insensitively and trimmed, so the same user signs in with another spelling
    let respelled = t.sign_in_raw(&format!(" {} ", f.member_a.email.to_uppercase()), &f.member_a.password).await;
    assert_eq!(respelled.status(), 200);
    // control: no credentials at all is still 401
    assert_eq!(t.http.get(auth_url(&t, "/session")).send().await.unwrap().status(), 401);
}

fn clone_user(u: &User) -> User {
    User { id: u.id, token: u.token.clone(), email: u.email.clone(), password: u.password.clone() }
}

/// ADR-0052: a mutation authenticated by the cookie needs the double-submit token. This is also spike 0003's K3: the
/// stock React app sends the token only on its auth form posts, so its JSON mutations would be refused here until its
/// service layer sends it (recorded in the research note, not decided).
#[tokio::test]
async fn a_cookie_authenticated_mutation_needs_the_csrf_token() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let s = t.sign_in(&f.member_a).await;
    let create = |csrf| as_browser(&t, Method::POST, t.url(&wi_path(&f.a)), &s, csrf).json(&json!({"name": "from a cookie"}));

    for (what, csrf) in [
        ("no token", Csrf::Absent),
        ("a token that is not the cookie's", Csrf::Wrong("0123456789abcdef0123456789abcdef")),
        ("an empty token", Csrf::Wrong("")),
        ("a token and no CSRF cookie", Csrf::NoCookie),
    ] {
        let r = create(csrf).send().await.unwrap();
        assert_eq!(r.status(), 403, "{what}");
        assert_eq!(error_code(r).await, "csrf_failed", "{what}");
    }
    // none of the refused requests did anything
    let before: Vec<WorkItem> = list(&t, &f.member_a, &f.a, None).await.json().await.unwrap();
    assert!(before.is_empty(), "a refused mutation created a row");
    // the same request with the token succeeds
    let ok = create(Csrf::Matching).send().await.unwrap();
    assert_eq!(ok.status(), 201, "the same mutation with the token");
    let created: WorkItem = ok.json().await.unwrap();
    assert_eq!(created.created_by, f.member_a.id, "created as the session's user");

    // controls: the token is only asked of a cookie-authenticated mutation. A bearer token is not an ambient credential,
    // so the same mutation without the token succeeds; a safe request with the cookie needs none.
    let bearer = create_ok(&t, &f.member_a, &f.a, "from a bearer token").await;
    assert_eq!(bearer.created_by, f.member_a.id);
    let safe = as_browser(&t, Method::GET, t.url(&wi_path(&f.a)), &s, Csrf::Absent).send().await.unwrap();
    assert_eq!(safe.status(), 200);
    // a delete is a mutation too
    let del = |csrf| as_browser(&t, Method::DELETE, t.url(&format!("{}/{}", wi_path(&f.a), created.id)), &s, csrf);
    assert_eq!(del(Csrf::Absent).send().await.unwrap().status(), 403, "a delete without the token");
    assert_eq!(del(Csrf::Matching).send().await.unwrap().status(), 204, "the same delete with it");
}

/// `GET /auth/csrf` (Plane's `get-csrf-token`): a fresh token each time, in the JSON body and in the CSRF cookie, with
/// the cookie's attributes, for a caller who has no session yet.
#[tokio::test]
async fn the_csrf_endpoint_hands_out_a_fresh_token_in_json_and_in_the_cookie() {
    let t = t();
    let mut seen = Vec::new();
    for _ in 0..2 {
        let r = t.http.get(auth_url(&t, "/csrf")).send().await.unwrap();
        assert_eq!(r.status(), 200);
        let cookies = SetCookie::all(&r);
        assert_eq!(cookies.len(), 1, "only the CSRF cookie: {cookies:?}");
        let c = &cookies[0];
        assert_eq!(c.name, CSRF_COOKIE);
        assert!(c.has_attr("Secure") && c.has_attr("HttpOnly") && c.attr("SameSite") == Some("Lax"));
        assert_eq!((c.attr("Path"), c.attr("Domain")), (Some("/"), None));
        let body: CsrfResponse = r.json().await.unwrap();
        assert!(is_hex(&body.csrf_token, 32), "{}", body.csrf_token);
        assert_eq!(body.csrf_token, c.value, "the JSON token is the cookie's");
        seen.push(body.csrf_token);
    }
    // control: two requests, two tokens (a constant token would be a CSRF token anyone can guess)
    assert_ne!(seen[0], seen[1]);
}

/// Sign-out ends one session and clears both cookies; another session of the same user lives on. It is a
/// cookie-authenticated mutation, so it needs the CSRF token too.
#[tokio::test]
async fn sign_out_ends_that_session_only() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let (s1, s2) = (t.sign_in(&f.member_a).await, t.sign_in(&f.member_a).await);
    assert_ne!(s1.token, s2.token, "every sign-in is a new session");
    let alive = |s: &Session| {
        let (t, f) = (&t, &f);
        let s = s.clone();
        async move {
            let u = User { token: s.token, ..clone_user(&f.member_a) };
            list(t, &u, &f.a, None).await.status() == 200
        }
    };
    assert!(alive(&s1).await && alive(&s2).await);

    // without the token it is refused, and the session is still there
    let r = as_browser(&t, Method::POST, auth_url(&t, "/sign-out"), &s1, Csrf::Absent).send().await.unwrap();
    assert_eq!(r.status(), 403);
    assert!(alive(&s1).await, "a refused sign-out must not end the session");

    // with it: 204, both cookies cleared, that session gone, the other not
    let r = as_browser(&t, Method::POST, auth_url(&t, "/sign-out"), &s1, Csrf::Matching).send().await.unwrap();
    assert_eq!(r.status(), 204);
    let cleared = SetCookie::all(&r);
    for name in [SESSION_COOKIE, CSRF_COOKIE] {
        let c = SetCookie::find(&cleared, name).unwrap_or_else(|| panic!("{name} not cleared: {cleared:?}"));
        assert_eq!((c.value.as_str(), c.attr("Max-Age")), ("", Some("0")), "{name}");
    }
    assert!(!alive(&s1).await, "the session is revoked on the next request");
    assert!(alive(&s2).await, "another session of the same user is not");
    // signing out again, or with nothing, is a 401 (no session to end)
    let again = as_browser(&t, Method::POST, auth_url(&t, "/sign-out"), &s1, Csrf::Matching).send().await.unwrap();
    assert_eq!(again.status(), 401);
    assert_eq!(t.http.post(auth_url(&t, "/sign-out")).send().await.unwrap().status(), 401);
    // a bearer token signs out without the CSRF token (it is not an ambient credential)
    let r = t.http.post(auth_url(&t, "/sign-out")).bearer_auth(&s2.token).send().await.unwrap();
    assert_eq!(r.status(), 204);
    assert!(!alive(&s2).await);
}

/// Sign-out everywhere ends every session of the user (the fixture's own included) and nobody else's.
#[tokio::test]
async fn sign_out_everywhere_ends_every_session_of_the_user_and_no_one_elses() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let mine: Vec<Session> = vec![t.sign_in(&f.member_a).await, t.sign_in(&f.member_a).await, t.sign_in(&f.member_a).await];
    let other = t.sign_in(&f.guest_a).await;
    let status = |token: &str, who: &User| {
        let u = User { token: token.to_string(), ..clone_user(who) };
        let (t, a) = (&t, &f.a);
        async move { list(t, &u, a, None).await.status() }
    };
    for s in &mine {
        assert_eq!(status(&s.token, &f.member_a).await, 200);
    }

    // refused without the CSRF token, and nothing ends
    let r = as_browser(&t, Method::POST, auth_url(&t, "/sign-out-all"), &mine[0], Csrf::Absent).send().await.unwrap();
    assert_eq!(r.status(), 403);
    assert_eq!(status(&mine[1].token, &f.member_a).await, 200);

    let r = as_browser(&t, Method::POST, auth_url(&t, "/sign-out-all"), &mine[0], Csrf::Matching).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let cleared = SetCookie::all(&r);
    assert!(cleared.iter().all(|c| c.value.is_empty() && c.attr("Max-Age") == Some("0")), "{cleared:?}");
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["revoked"], 4, "three sign-ins and the fixture's own session");
    for s in &mine {
        assert_eq!(status(&s.token, &f.member_a).await, 401, "a session of the user survived");
    }
    assert_eq!(status(&f.member_a.token, &f.member_a).await, 401, "the fixture's session too");
    // controls: another user's sessions are untouched, and the user can sign in again
    assert_eq!(status(&other.token, &f.guest_a).await, 200);
    assert_eq!(status(&f.guest_a.token, &f.guest_a).await, 200);
    let again = t.sign_in(&f.member_a).await;
    assert_eq!(status(&again.token, &f.member_a).await, 200);
}

/// ADR-0052 (Tests): a deactivated user's session is rejected on the very next request, and they cannot sign in;
/// reactivating brings the session back. (Deactivation is a test route until ADR-0053.)
#[tokio::test]
async fn a_deactivated_users_session_is_rejected() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let s = t.sign_in(&f.member_a).await;
    let who = |token: &str| User { token: token.to_string(), ..clone_user(&f.member_a) };
    assert_eq!(list(&t, &who(&s.token), &f.a, None).await.status(), 200, "before");
    t.set_user_active(f.member_a.id, false).await;
    assert_eq!(list(&t, &who(&s.token), &f.a, None).await.status(), 401, "the signed-in session");
    assert_eq!(list(&t, &f.member_a, &f.a, None).await.status(), 401, "the fixture's session");
    let cookie = as_browser(&t, Method::GET, auth_url(&t, "/session"), &s, Csrf::Absent).send().await.unwrap();
    assert_eq!(cookie.status(), 401, "the cookie");
    assert_eq!(t.sign_in_raw(&f.member_a.email, &f.member_a.password).await.status(), 401, "no new sign-in");
    // controls: nobody else is affected, and reactivation undoes it
    assert_eq!(list(&t, &f.admin_a, &f.a, None).await.status(), 200, "another user");
    assert_eq!(t.sign_in_raw(&f.admin_a.email, &f.admin_a.password).await.status(), 200);
    t.set_user_active(f.member_a.id, true).await;
    assert_eq!(list(&t, &who(&s.token), &f.a, None).await.status(), 200, "reactivated");
    assert_eq!(t.sign_in_raw(&f.member_a.email, &f.member_a.password).await.status(), 200);
    // a typo in a test is not a silent success
    let r = t.admin_post("/internal/set-user-active", &json!({ "user_id": UserId(uuid::Uuid::new_v4()), "active": false })).await;
    assert_eq!(r.status(), 404);
}
