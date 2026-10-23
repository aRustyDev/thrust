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

// ---------------------------------------------------------------------------------------------------------------
// API keys (ADR-0052 K1, ADR-0033 round 27): `X-Api-Key`, shown once, stored hashed. Statuses reproduce the oracle:
// only a MISSING key is 401; an invalid, expired, revoked or inactive-user key is 403.
// ---------------------------------------------------------------------------------------------------------------

const API_KEY_HEADER: &str = "x-api-key";

fn keys_url(t: &Target) -> String {
    t.url(&format!("{SLICE_PREFIX}/api-keys"))
}

async fn create_key(t: &Target, who: &User, body: Value) -> reqwest::Response {
    t.http.post(keys_url(t)).bearer_auth(&who.token).json(&body).send().await.unwrap()
}

async fn create_key_ok(t: &Target, who: &User, body: Value) -> CreatedApiKey {
    let r = create_key(t, who, body).await;
    assert_eq!(r.status(), 201, "create key");
    r.json().await.unwrap()
}

/// The status of a work-item list made with `X-Api-Key: key` (a header value that is `None` sends no header).
async fn list_with_key(t: &Target, at: &At, key: Option<&str>) -> u16 {
    let mut r = t.http.get(t.url(&wi_path(at)));
    if let Some(k) = key {
        r = r.header(API_KEY_HEADER, k);
    }
    r.send().await.unwrap().status().as_u16()
}

async fn seed_keys(t: &Target, keys: Vec<SeedApiKey>) {
    let f = SeedFixture { api_keys: keys, ..SeedFixture::default() };
    let r = t.admin_post("/internal/seed", &serde_json::to_value(&f).unwrap()).await;
    assert_eq!(r.status(), 204, "seed keys: {:?}", r.text().await);
}

/// A key is a token with a visible prefix, returned once, accepted as `X-Api-Key` for reads and mutations without a
/// CSRF token, and never shown again; a session is needed to manage keys, a key is not.
#[tokio::test]
async fn an_api_key_is_shown_once_and_authenticates_as_x_api_key() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let r = create_key(&t, &f.member_a, json!({ "label": "ci" })).await;
    assert_eq!(r.status(), 201);
    assert_eq!(r.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-store"));
    let text = r.text().await.unwrap();
    let key: CreatedApiKey = serde_json::from_str(&text).unwrap();
    let secret = key.token.strip_prefix("thrk_").expect("the visible marker");
    assert!(is_hex(secret, 64), "256 random bits, hex: {}", key.token);
    assert_eq!((key.prefix.as_str(), key.label.as_str()), (&key.token[..9], "ci"));

    // it authenticates, for a read and for a mutation (no CSRF token: a key is not an ambient credential), as its owner
    assert_eq!(list_with_key(&t, &f.a, Some(&key.token)).await, 200);
    let r = t.http.post(t.url(&wi_path(&f.a))).header(API_KEY_HEADER, &key.token).json(&json!({"name": "by key"})).send().await.unwrap();
    assert_eq!(r.status(), 201);
    assert_eq!(r.json::<WorkItem>().await.unwrap().created_by, f.member_a.id);
    let me = t.http.get(auth_url(&t, "/session")).header(API_KEY_HEADER, &key.token).send().await.unwrap();
    assert_eq!(me.json::<SessionInfo>().await.unwrap(), SessionInfo { user_id: f.member_a.id, credential: CredentialKind::ApiKey });
    // it carries its owner's rights and no more: a guest's key cannot write
    let guest_key = create_key_ok(&t, &f.guest_a, json!({})).await;
    let r = t.http.post(t.url(&wi_path(&f.a))).header(API_KEY_HEADER, &guest_key.token).json(&json!({"name": "x"})).send().await.unwrap();
    assert_eq!(r.status(), 403, "a guest's key is a guest");
    assert_eq!(list_with_key(&t, &f.a, Some(&guest_key.token)).await, 200);

    // listed without the token, with the prefix; `last_used_at` is stamped after the response
    let r = t.http.get(keys_url(&t)).bearer_auth(&f.member_a.token).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let text = r.text().await.unwrap();
    assert!(!text.contains(secret) && !text.contains(&key.token), "the listing shows the token: {text}");
    let listed: Vec<ApiKeyInfo> = serde_json::from_str(&text).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].id, listed[0].prefix.as_str(), listed[0].label.as_str()), (key.id, key.prefix.as_str(), "ci"));
    eventually("the key's last use is recorded", std::time::Duration::from_secs(15), || async {
        let r = t.http.get(keys_url(&t)).bearer_auth(&f.member_a.token).send().await.unwrap();
        let keys: Vec<ApiKeyInfo> = r.json().await.unwrap();
        keys[0].last_used_at.map(|_| ())
    })
    .await;
    // another user's listing has none of it
    let theirs: Vec<ApiKeyInfo> = t.http.get(keys_url(&t)).bearer_auth(&f.admin_a.token).send().await.unwrap().json().await.unwrap();
    assert!(theirs.is_empty());

    // controls: a missing key is the only 401; an unknown one, and an empty header, are not what they look like
    assert_eq!(list_with_key(&t, &f.a, None).await, 401, "no key");
    assert_eq!(list_with_key(&t, &f.a, Some("")).await, 401, "an empty header is no key");
    assert_eq!(list_with_key(&t, &f.a, Some("thrk_not-a-key")).await, 403, "an unknown key");
    // control: a key manages nothing: it cannot mint keys, list them, or end the owner's sessions
    let with_key = |m: Method, url: String| t.http.request(m, url).header(API_KEY_HEADER, &key.token);
    assert_eq!(with_key(Method::POST, keys_url(&t)).json(&json!({})).send().await.unwrap().status(), 401, "a key mints no key");
    assert_eq!(with_key(Method::GET, keys_url(&t)).send().await.unwrap().status(), 401);
    assert_eq!(with_key(Method::POST, auth_url(&t, "/sign-out-all")).send().await.unwrap().status(), 401);
    assert_eq!(list(&t, &f.member_a, &f.a, None).await.status(), 200, "the owner's session was not ended");
    // control: minting a key with the cookie is a mutation, so it needs the CSRF token
    let s = t.sign_in(&f.admin_a).await;
    let r = as_browser(&t, Method::POST, keys_url(&t), &s, Csrf::Absent).json(&json!({})).send().await.unwrap();
    assert_eq!(r.status(), 403);
    let r = as_browser(&t, Method::POST, keys_url(&t), &s, Csrf::Matching).json(&json!({})).send().await.unwrap();
    assert_eq!(r.status(), 201);
    // a bad request is a 400, not a key
    let past = serde_json::to_value(CreateApiKeyRequest { label: None, expires_at: Some(thrust_contracts::chrono::DateTime::from_timestamp(1_000_000_000, 0).unwrap()) }).unwrap();
    assert_eq!(create_key(&t, &f.member_a, past).await.status(), 400, "an expiry in the past");
    assert_eq!(create_key(&t, &f.member_a, json!({ "label": "y".repeat(101) })).await.status(), 400, "a label of 101 characters");
}

/// ADR-0052 (Tests) with the status corrected by this issue: a revoked key is rejected on the NEXT request with Plane's
/// status, 403 (the oracle answers 403 for an invalid, expired, revoked or inactive-user key, ADR-0033 round 27; ADR-0052
/// said 401), while an unrevoked key still gets 200.
#[tokio::test]
async fn a_revoked_key_is_rejected_on_the_next_request_with_403() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let (a, b) = (create_key_ok(&t, &f.member_a, json!({})).await, create_key_ok(&t, &f.member_a, json!({})).await);
    let theirs = create_key_ok(&t, &f.admin_a, json!({})).await;
    for k in [&a, &b, &theirs] {
        assert_eq!(list_with_key(&t, &f.a, Some(&k.token)).await, 200, "before the revoke");
    }
    let delete = |who: &User, id: ApiKeyId| {
        let url = format!("{}/{id}", keys_url(&t));
        let r = t.http.delete(url).bearer_auth(&who.token);
        async move { r.send().await.unwrap().status().as_u16() }
    };
    // someone else cannot revoke it (404: nothing to learn about other users' keys), and it keeps working
    assert_eq!(delete(&f.admin_a, a.id).await, 404);
    assert_eq!(list_with_key(&t, &f.a, Some(&a.token)).await, 200, "another user's revoke must not end the key");
    assert_eq!(delete(&f.member_a, ApiKeyId(uuid::Uuid::new_v4())).await, 404, "an unknown id");
    // the owner revokes it: 403 from the next request on, with Plane's status and not 401
    assert_eq!(delete(&f.member_a, a.id).await, 204);
    assert_eq!(list_with_key(&t, &f.a, Some(&a.token)).await, 403, "the revoked key");
    let r = t.http.post(t.url(&wi_path(&f.a))).header(API_KEY_HEADER, &a.token).json(&json!({"name": "x"})).send().await.unwrap();
    assert_eq!(r.status(), 403, "a mutation with it");
    assert_eq!(error_code(r).await, "invalid_api_key");
    // controls: the owner's other key and another user's key are unaffected (so the 403 is the revoke, not the route)
    assert_eq!(list_with_key(&t, &f.a, Some(&b.token)).await, 200, "an unrevoked key of the same user");
    assert_eq!(list_with_key(&t, &f.a, Some(&theirs.token)).await, 200, "another user's key");
    // revoking again is a 204, and the listing shows both keys with the first revocation
    assert_eq!(delete(&f.member_a, a.id).await, 204);
    let listed: Vec<ApiKeyInfo> = t.http.get(keys_url(&t)).bearer_auth(&f.member_a.token).send().await.unwrap().json().await.unwrap();
    assert_eq!(listed.iter().map(|k| (k.id, k.revoked_at.is_some())).collect::<Vec<_>>(), [(a.id, true), (b.id, false)]);
    // the key's own request did not revive anything: still 403
    assert_eq!(list_with_key(&t, &f.a, Some(&a.token)).await, 403);
}

/// An expired key and a deactivated user's key are 403 too, and the live ones next to them are 200.
#[tokio::test]
async fn an_expired_key_and_a_deactivated_users_key_are_rejected_with_403() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let token = |n: &str| format!("thrk_{n}-{}", uuid::Uuid::new_v4().simple());
    let (expired, live, forever, inactive_owner) = (token("expired"), token("live"), token("forever"), token("inactive"));
    let in_a_day = thrust_contracts::chrono::DateTime::from_timestamp(4_000_000_000, 0).unwrap();
    let in_the_past = thrust_contracts::chrono::DateTime::from_timestamp(1_000_000_000, 0).unwrap();
    let key = |token: &String, user: &User, expires_at| SeedApiKey { token: token.clone(), user_id: user.id, label: "seeded".into(), expires_at, revoked: false };
    seed_keys(
        &t,
        vec![
            key(&expired, &f.member_a, Some(in_the_past)),
            key(&live, &f.member_a, Some(in_a_day)),
            key(&forever, &f.member_a, None),
            key(&inactive_owner, &f.guest_a, None),
        ],
    )
    .await;
    assert_eq!(list_with_key(&t, &f.a, Some(&expired)).await, 403, "an expired key");
    assert_eq!(list_with_key(&t, &f.a, Some(&live)).await, 200, "control: a key that expires later");
    assert_eq!(list_with_key(&t, &f.a, Some(&forever)).await, 200, "control: a key that never expires");

    assert_eq!(list_with_key(&t, &f.a, Some(&inactive_owner)).await, 200, "before the deactivation");
    t.set_user_active(f.guest_a.id, false).await;
    assert_eq!(list_with_key(&t, &f.a, Some(&inactive_owner)).await, 403, "a deactivated user's key");
    assert_eq!(list_with_key(&t, &f.a, Some(&live)).await, 200, "control: another user's key");
    t.set_user_active(f.guest_a.id, true).await;
    assert_eq!(list_with_key(&t, &f.a, Some(&inactive_owner)).await, 200, "reactivated");
}

// ---------------------------------------------------------------------------------------------------------------
// OIDC: one authorization-code login with PKCE through the `openidconnect` crate, against the mock identity provider
// the suite serves on 127.0.0.1 (`thrust_contract_tests::mock_idp`). The mock checks the authorization request the
// target built and the PKCE verifier it sends, and signs ID tokens with the `rsa` crate, so the two sides share no code.
// A deployed Worker cannot reach 127.0.0.1: these `oidc_` tests are skipped by scripts/stage2-contract.sh.
// ---------------------------------------------------------------------------------------------------------------

use thrust_contract_tests::mock_idp::{Callback, Identity, MockIdp, Tamper};

fn unique_sub() -> String {
    format!("sub-{}", uuid::Uuid::new_v4().simple())
}

/// `GET /auth/oidc/start` as a browser: a 302 whose Location the caller takes to the provider.
async fn oidc_start(t: &Target) -> String {
    let r = t.no_redirect().get(auth_url(t, "/oidc/start")).send().await.unwrap();
    assert_eq!(r.status(), 302, "start answers a redirect");
    r.headers().get("location").and_then(|v| v.to_str().ok()).expect("a Location").to_string()
}

/// The whole front half: start, then the provider logs `identity` in.
async fn begin(t: &Target, identity: Identity, tamper: Tamper) -> Callback {
    // the provider first: the target discovers it while it handles `start`
    let idp = MockIdp::shared();
    let location = oidc_start(t).await;
    idp.authorize(&location, identity, tamper)
}

async fn finish(t: &Target, cb: &Callback) -> reqwest::Response {
    t.no_redirect().get(cb.url()).send().await.unwrap()
}

/// A callback that was accepted: 303 to the root with the session and CSRF cookies, which are a working session.
async fn assert_logged_in(t: &Target, r: reqwest::Response, who: UserId) -> Session {
    assert_eq!(r.status(), 303);
    assert_eq!(r.headers().get("location").and_then(|v| v.to_str().ok()), Some("/"));
    let cookies = SetCookie::all(&r);
    let session = SetCookie::find(&cookies, SESSION_COOKIE).expect("a session cookie");
    let csrf = SetCookie::find(&cookies, CSRF_COOKIE).expect("a csrf cookie");
    assert!(is_hex(&session.value, 64) && is_hex(&csrf.value, 32));
    for c in [session, csrf] {
        assert!(c.has_attr("Secure") && c.has_attr("HttpOnly") && c.attr("SameSite") == Some("Lax"), "{}", c.name);
        assert_eq!((c.attr("Path"), c.attr("Domain"), c.attr("Max-Age")), (Some("/"), None, Some("2592000")), "{}", c.name);
    }
    let s = Session { user_id: who, token: session.value.clone(), csrf: csrf.value.clone() };
    let me = as_browser(t, Method::GET, auth_url(t, "/session"), &s, Csrf::Absent).send().await.unwrap();
    assert_eq!(me.status(), 200, "the cookie from the callback is a session");
    assert_eq!(me.json::<SessionInfo>().await.unwrap().user_id, who);
    s
}

/// A callback that was refused: the status, and no cookie, whatever else.
async fn assert_refused(r: reqwest::Response, status: u16, what: &str) {
    assert_eq!(r.status().as_u16(), status, "{what}");
    assert!(SetCookie::all(&r).is_empty(), "{what}: a refused login must not set a cookie");
}

/// ADR-0050, spike 0003 step 4: the login completes through `openidconnect` and ends in the same session row and
/// cookies as a password sign-in. The first login of a user known by email links the provider's subject; from then on
/// the subject decides, so another user's email cannot take the account over.
#[tokio::test]
async fn oidc_login_completes_and_ends_in_the_same_session_as_a_password_sign_in() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let sub = unique_sub();

    // verified email of a user with no link yet: the subject is linked and the user is logged in
    let cb = begin(&t, Identity::verified(&sub, &f.member_a.email), Tamper::None).await;
    let s = assert_logged_in(&t, finish(&t, &cb).await, f.member_a.id).await;
    assert_eq!(list(&t, &User { token: s.token.clone(), ..clone_user(&f.member_a) }, &f.a, None).await.status(), 200);

    // the link now decides: the same subject reporting another user's verified email is still the first user
    let cb = begin(&t, Identity::verified(&sub, &f.admin_a.email), Tamper::None).await;
    assert_logged_in(&t, finish(&t, &cb).await, f.member_a.id).await;
    // control: a different subject with that email is a different login, and goes to that user
    let cb = begin(&t, Identity::verified(&unique_sub(), &f.admin_a.email), Tamper::None).await;
    assert_logged_in(&t, finish(&t, &cb).await, f.admin_a.id).await;
    // the email is matched like a password sign-in's: case and padding do not matter
    let cb = begin(&t, Identity::verified(&unique_sub(), &format!(" {} ", f.guest_a.email.to_uppercase())), Tamper::None).await;
    assert_logged_in(&t, finish(&t, &cb).await, f.guest_a.id).await;

    // control for the mock itself (rule 8): it refuses an exchange whose PKCE verifier is not the challenge's. Without
    // this, nothing here would show that the target's own, correct verifier is what made the logins above work.
    let cb = begin(&t, Identity::verified(&unique_sub(), &f.member_a.email), Tamper::None).await;
    let (status, body) = MockIdp::shared().exchange_with_verifier(&cb, &"0".repeat(64)).await;
    assert_eq!((status, body["error"].as_str()), (400, Some("invalid_grant")), "{body}");
}

/// A callback whose `state` is not the one this server issued is refused before the provider is asked anything, and the
/// real callback of the same login still completes afterwards (the bad one used nothing up).
#[tokio::test]
async fn oidc_a_tampered_state_is_rejected() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let cb = begin(&t, Identity::verified(&unique_sub(), &f.member_a.email), Tamper::None).await;
    let flipped = if cb.state.ends_with('0') { '1' } else { '0' };
    let tampered = Callback { state: format!("{}{flipped}", &cb.state[..63]), ..cb.clone() };
    assert_refused(finish(&t, &tampered).await, 400, "a state with one character changed").await;
    let unknown = Callback { state: "f".repeat(64), ..cb.clone() };
    assert_refused(finish(&t, &unknown).await, 400, "a state nobody issued").await;
    let none = t.no_redirect().get(t.url(&format!("{SLICE_PREFIX}/auth/oidc/callback"))).send().await.unwrap();
    assert_refused(none, 400, "no code and no state").await;
    let denied = t.no_redirect().get(format!("{}?error=access_denied&state={}", cb.redirect_uri, cb.state)).send().await.unwrap();
    assert_refused(denied, 401, "the provider reports an error").await;
    // control: none of that used the login up; the genuine callback completes
    assert_logged_in(&t, finish(&t, &cb).await, f.member_a.id).await;
}

/// A callback is good once: the stored `state` is deleted when it is taken, so repeating it finds nothing.
#[tokio::test]
async fn oidc_a_replayed_callback_is_rejected() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let cb = begin(&t, Identity::verified(&unique_sub(), &f.member_a.email), Tamper::None).await;
    assert_logged_in(&t, finish(&t, &cb).await, f.member_a.id).await;
    assert_refused(finish(&t, &cb).await, 400, "the same callback again").await;
    // control: a fresh login is unaffected
    let cb = begin(&t, Identity::verified(&unique_sub(), &f.member_a.email), Tamper::None).await;
    assert_logged_in(&t, finish(&t, &cb).await, f.member_a.id).await;
}

/// The target verifies the ID token the provider returns: a wrong `nonce`, another audience or issuer, an expired
/// token and a corrupted signature are each refused (401, no cookie). On the Worker this is `openidconnect`'s RS256
/// verification running as wasm32.
#[tokio::test]
async fn oidc_an_id_token_that_does_not_verify_is_rejected() {
    let t = t();
    let f = Fixture::seed(&t).await;
    for tamper in [Tamper::WrongNonce, Tamper::WrongAudience, Tamper::WrongIssuer, Tamper::Expired, Tamper::BadSignature] {
        let cb = begin(&t, Identity::verified(&unique_sub(), &f.member_a.email), tamper).await;
        assert_refused(finish(&t, &cb).await, 401, &format!("{tamper:?}")).await;
    }
    // control: the same login with an honest token is accepted, so each refusal above was the tampering
    let cb = begin(&t, Identity::verified(&unique_sub(), &f.member_a.email), Tamper::None).await;
    assert_logged_in(&t, finish(&t, &cb).await, f.member_a.id).await;
}

/// ADR-0050: a verified email is required (the guard Plane added after GHSA-7j95-vh8g-f365). A provider that says the
/// email is not verified, says nothing about it, or gives none does not log anyone in, even when the email is a user's.
#[tokio::test]
async fn oidc_an_unverified_or_missing_email_is_rejected() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let email = f.member_a.email.clone();
    for (what, identity) in [
        ("email_verified false", Identity { sub: unique_sub(), email: Some(email.clone()), email_verified: Some(false) }),
        ("email_verified absent", Identity { sub: unique_sub(), email: Some(email.clone()), email_verified: None }),
        ("no email", Identity { sub: unique_sub(), email: None, email_verified: Some(true) }),
    ] {
        let cb = begin(&t, identity, Tamper::None).await;
        assert_refused(finish(&t, &cb).await, 403, what).await;
    }
    // an already linked subject does not get around it
    let sub = unique_sub();
    let cb = begin(&t, Identity::verified(&sub, &email), Tamper::None).await;
    assert_logged_in(&t, finish(&t, &cb).await, f.member_a.id).await;
    let cb = begin(&t, Identity { sub, email: Some(email.clone()), email_verified: Some(false) }, Tamper::None).await;
    assert_refused(finish(&t, &cb).await, 403, "a linked subject with an unverified email").await;
    // control: verified is accepted
    let cb = begin(&t, Identity::verified(&unique_sub(), &email), Tamper::None).await;
    assert_logged_in(&t, finish(&t, &cb).await, f.member_a.id).await;
}

/// No just-in-time provisioning at P0 (Q-0050.1): a verified email that no user has is refused, and creates nobody. A
/// deactivated user cannot log in through the provider either.
#[tokio::test]
async fn oidc_there_is_no_provisioning_and_a_deactivated_user_cannot_log_in() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let stranger = format!("stranger-{}@example.test", uuid::Uuid::new_v4().simple());
    let cb = begin(&t, Identity::verified(&unique_sub(), &stranger), Tamper::None).await;
    assert_refused(finish(&t, &cb).await, 403, "a verified email nobody has").await;
    assert_eq!(t.sign_in_raw(&stranger, "anything").await.status(), 401, "and no user was made");

    let sub = unique_sub();
    t.set_user_active(f.guest_a.id, false).await;
    let cb = begin(&t, Identity::verified(&sub, &f.guest_a.email), Tamper::None).await;
    assert_refused(finish(&t, &cb).await, 403, "a deactivated user, by email").await;
    // control: reactivated, the same login is accepted, and then the link makes the deactivated state matter by subject
    t.set_user_active(f.guest_a.id, true).await;
    let cb = begin(&t, Identity::verified(&sub, &f.guest_a.email), Tamper::None).await;
    assert_logged_in(&t, finish(&t, &cb).await, f.guest_a.id).await;
    t.set_user_active(f.guest_a.id, false).await;
    let cb = begin(&t, Identity::verified(&sub, &f.guest_a.email), Tamper::None).await;
    assert_refused(finish(&t, &cb).await, 403, "a deactivated user, by link").await;
}
