//! The password reset by email on a server that has a mail service (E2, D-041): the real `thrust-server` started with `THRUST_SMTP_*` pointing at a
//! Mailpit, and `THRUST_PUBLIC_BASE_URL` set. `scripts/contract.sh <native-target> mail` starts both and tells these tests where the mail is read
//! (`THRUST_MAIL_API_URL`). They are the one place the whole path is walked on a running server: the request, the SMTP conversation, the message a
//! mail server reads, the link in it, and the reset it leads to. What each piece does on its own is the adapters' and core's tests; what the
//! composition root wired, and that the pieces agree with each other, only shows here.
//!
//! Mail is not a contract target on the other three (no Worker binding yet, E3), so this is its own test binary and its own pass: the main pass
//! runs against targets with no mail, and holds that the route is dark there (`compat_forgot_password_is_dark_on_a_target_with_no_mail_service`).

use reqwest::Url;
use serde_json::{json, Value};
use thrust_contract_tests::*;

const STRONG: &str = "Contract-Sample-Pa55word-Zebra-81";
const MAILED: &str = "Mailed-Reset-Pa55word-Otter-93-Contract";
const SENDER: &str = "no-reply@thrust.test";
const URLENCODED: &str = "application/x-www-form-urlencoded";

fn t() -> Target {
    Target::from_env()
}

fn mail_api() -> String {
    std::env::var("THRUST_MAIL_API_URL").expect(
        "THRUST_MAIL_API_URL must be set: scripts/contract.sh <native target> mail starts the mail server and sets it",
    )
}

fn public_base() -> String {
    std::env::var("THRUST_PUBLIC_BASE_URL")
        .expect("THRUST_PUBLIC_BASE_URL must be set: scripts/contract.sh <native target> mail sets it")
        .trim_end_matches('/')
        .to_string()
}

fn new_email() -> String {
    format!("mail-{}@example.test", uuid::Uuid::new_v4().simple())
}

async fn json_of(url: &str, query: &[(&str, String)]) -> Value {
    reqwest::Client::new().get(url).query(query).send().await.unwrap().json().await.unwrap()
}

/// Every message the mail server holds for `address`, in full, oldest first.
async fn mails_to(address: &str) -> Vec<Value> {
    let api = mail_api();
    let found = json_of(&format!("{api}/api/v1/search"), &[("query", format!("to:{address}"))]).await;
    let mut all = Vec::new();
    for m in found["messages"].as_array().cloned().unwrap_or_default().iter().rev() {
        all.push(json_of(&format!("{api}/api/v1/message/{}", m["ID"].as_str().unwrap()), &[]).await);
    }
    all
}

async fn sign_up(t: &Target) -> (String, String) {
    let email = new_email();
    let r =
        t.http.post(t.url("/auth/sign-up/")).json(&json!({"email": email, "password": STRONG})).send().await.unwrap();
    assert_eq!(r.status(), 200, "sign-up");
    let cookies = SetCookie::all(&r);
    let session = SetCookie::find(&cookies, SESSION_COOKIE).expect("a sign-up signs the account in").value.clone();
    (email, session)
}

async fn forgot(t: &Target, body: &Value) -> reqwest::Response {
    t.http.post(t.url("/auth/forgot-password/")).json(body).send().await.unwrap()
}

async fn sign_in_status(t: &Target, email: &str, password: &str) -> u16 {
    t.http
        .post(t.url("/auth/sign-in/"))
        .json(&json!({"email": email, "password": password}))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn reset_form(t: &Target, uidb64: &str, token: &str, password: &str) -> reqwest::Response {
    t.no_redirect()
        .post(t.url(&format!("/auth/reset-password/{uidb64}/{token}/")))
        .header("content-type", URLENCODED)
        .body(format!("password={password}"))
        .send()
        .await
        .unwrap()
}

fn location(r: &reqwest::Response) -> String {
    r.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string()
}

/// The link of a reset mail, as the person would open it: the one address in the text that starts with the base the deployment is configured with.
fn link_in(mail: &Value) -> Url {
    let text = mail["Text"].as_str().expect("a text part");
    let word =
        text.split_whitespace().find(|w| w.starts_with("http")).unwrap_or_else(|| panic!("a link in the mail: {text}"));
    Url::parse(word).unwrap_or_else(|e| panic!("the link parses ({e}): {word}"))
}

/// The app draws the link only when `/api/instances/` says mail is on, and a target with a mail service and a base URL says so.
#[tokio::test]
async fn mail_the_instance_says_mail_is_on_when_the_server_has_a_relay_and_a_base_url() {
    let t = t();
    let body: Value = t.http.get(t.url("/api/instances/")).send().await.unwrap().json().await.unwrap();
    assert_eq!(body["config"]["is_smtp_configured"], true, "{body}");
}

/// The whole path. A person who forgot the password asks; one message arrives at a mail server that reads it for itself; its link starts with the
/// configured base and carries the account and a token; the reset page's form with them sets the new password, ends the session, and uses the
/// link up. A token one character off is refused first and changes nothing, so it is the mailed token and not any token that works.
#[tokio::test]
async fn mail_a_forgotten_password_is_reset_through_the_link_in_the_mail() {
    let t = t();
    let (email, session) = sign_up(&t).await;
    assert!(mails_to(&email).await.is_empty(), "no mail before anything was asked");

    let asked = forgot(&t, &json!({"email": email})).await;
    assert_eq!(asked.status(), 200, "{:?}", asked.text().await);

    let mails = mails_to(&email).await;
    assert_eq!(mails.len(), 1, "one request, one message");
    let mail = &mails[0];
    assert_eq!(mail["Subject"], "Reset your Thrust password");
    assert_eq!(mail["From"]["Address"], SENDER);
    assert_eq!(mail["To"].as_array().unwrap().len(), 1);
    assert_eq!(mail["To"][0]["Address"], email.as_str());
    assert!(mail["Text"].as_str().unwrap().contains(&email), "the message names the account");

    let link = link_in(mail);
    assert!(
        link.as_str().starts_with(&public_base()),
        "the link starts with the configured base, {}: {link}",
        public_base()
    );
    assert_eq!(link.path(), "/accounts/reset-password/");
    let q: std::collections::HashMap<String, String> = link.query_pairs().into_owned().collect();
    assert_eq!(q.get("email"), Some(&email), "the address is in the link, decoded as the app reads it");
    let (uidb64, token) = (q["uidb64"].clone(), q["token"].clone());
    assert!(token.len() >= 40, "a token of 256 bits is not short: {token}");

    // CONTROL: a token one character off is refused, and the password is as it was
    let mut off = token.clone();
    off.pop();
    off.push(if token.ends_with('a') { 'b' } else { 'a' });
    let r = reset_form(&t, &uidb64, &off, MAILED).await;
    assert_eq!(r.status(), 302);
    assert_eq!(location(&r), "/accounts/reset-password?error_code=5125&error_message=INVALID_PASSWORD_TOKEN");
    assert_eq!(sign_in_status(&t, &email, STRONG).await, 200, "a refused token changed nothing");

    // the link as mailed
    let r = reset_form(&t, &uidb64, &token, MAILED).await;
    assert_eq!(r.status(), 302);
    assert_eq!(location(&r), "/sign-in?success=True");
    assert!(SetCookie::all(&r).is_empty(), "a reset signs nobody in");
    assert_eq!(t.me_status(&session).await, 401, "the session held before it is over");
    assert_eq!(sign_in_status(&t, &email, STRONG).await, 400, "the old password is refused");
    assert_eq!(sign_in_status(&t, &email, MAILED).await, 200, "and the new one signs in");

    // used up
    let again = reset_form(&t, &uidb64, &token, "Another-Strong-Pa55word-3").await;
    assert_eq!(location(&again), "/accounts/reset-password?error_code=5125&error_message=INVALID_PASSWORD_TOKEN");
    assert_eq!(sign_in_status(&t, &email, "Another-Strong-Pa55word-3").await, 400);
    assert_eq!(mails_to(&email).await.len(), 1, "the reset sent no further mail");
}

/// What sends nothing: an address nobody has and one that is not an address are the pin's answers and no mail; a second request inside the
/// cooldown is answered as the first was and mails nothing (D-041, 2); and the link starts with the configured base whatever the request's `Host`
/// says (D-041, 4: a forged one would mail a victim a working token to somebody else's page). Each "nothing" is beside a request that did send.
#[tokio::test]
async fn mail_what_sends_nothing_and_what_the_link_ignores() {
    let t = t();
    let (email, _) = sign_up(&t).await;

    let unknown = forgot(&t, &json!({"email": new_email()})).await;
    assert_eq!(unknown.status(), 400);
    assert_eq!(unknown.json::<Value>().await.unwrap()["error_code"], 5060);
    let bad = forgot(&t, &json!({"email": "not an address"})).await;
    assert_eq!(bad.status(), 400);
    assert_eq!(bad.json::<Value>().await.unwrap()["error_code"], 5005);

    // the first request names a forged Host and is answered; the link in its mail is still the configured base
    let first = t
        .http
        .post(t.url("/auth/forgot-password/"))
        .header("host", "evil.example")
        .header("x-forwarded-host", "evil.example")
        .json(&json!({"email": email}))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let after_first = mails_to(&email).await;
    assert_eq!(after_first.len(), 1, "the control: this request did send");
    let link = link_in(&after_first[0]);
    assert!(link.as_str().starts_with(&public_base()), "the link followed the request's Host: {link}");
    assert!(!link.as_str().contains("evil.example"), "{link}");

    // a second one, inside the cooldown, is answered the same and mails nothing
    let second = forgot(&t, &json!({"email": email})).await;
    assert_eq!(second.status(), 200, "answered as one that mailed");
    assert_eq!(second.json::<Value>().await.unwrap(), json!({"message": "Check your email to reset your password"}));
    // no wait is needed to see a message that was sent: the mail goes in the request (D-041, 5), so the relay held it before the answer came
    assert_eq!(mails_to(&email).await.len(), 1, "a request inside the cooldown mailed something");
}
