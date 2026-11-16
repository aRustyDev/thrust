//! The password reset by email on a target that has a mail service (D-041), whichever kind it is:
//!
//! * a native target (E2): the real `thrust-server` started with `THRUST_SMTP_*` pointing at a Mailpit, and `THRUST_PUBLIC_BASE_URL` set.
//!   `scripts/contract.sh <native-target> mail` starts both and tells these tests where the mail is read (`THRUST_MAIL_API_URL`);
//! * a Worker target (E3): the Worker, built for wasm32 and run by `wrangler dev`, started with `THRUST_EMAIL_FROM` and
//!   `THRUST_PUBLIC_BASE_URL`. Its `send_email` binding is wrangler's simulator, which sends nothing and writes each message it is asked to
//!   send to the dev server's log and to files (`THRUST_MAIL_WRANGLER_LOG`, the log). The adapter that is run is the Worker's own, with the
//!   binding's real JS shape; what is not run is the service behind it, which only a deployed Worker reaches (G-39).
//!
//! They are the one place the whole path is walked on a running target: the request, the message the mail service was given, the link in it,
//! and the reset it leads to. What each piece does on its own is the adapters' and core's tests; what the composition root wired, and that the
//! pieces agree with each other, only shows here. The same three tests run against both kinds: the Worker's log is read into the shape
//! Mailpit's API has (`Subject`, `From`, `To`, `Text`), so that what is asserted is one thing.
//!
//! Mail is its own test binary and its own pass: the main pass runs against targets with no mail, and holds that the route is dark there
//! (`compat_forgot_password_is_dark_on_a_target_with_no_mail_service`).

use reqwest::Url;
use serde_json::{json, Value};
use thrust_contract_tests::*;
use thrust_contracts::UserId;

const STRONG: &str = "Contract-Sample-Pa55word-Zebra-81";
const MAILED: &str = "Mailed-Reset-Pa55word-Otter-93-Contract";
const SENDER: &str = "no-reply@thrust.test";
const URLENCODED: &str = "application/x-www-form-urlencoded";

fn t() -> Target {
    Target::from_env()
}

/// Where the mail is read: a mail server's API (a native target's Mailpit), or the log of the Worker's simulated `send_email` binding.
enum Source {
    Api(String),
    SimulatorLog(String),
}

fn source() -> Source {
    match (std::env::var("THRUST_MAIL_API_URL"), std::env::var("THRUST_MAIL_WRANGLER_LOG")) {
        (Ok(api), Err(_)) => Source::Api(api),
        (Err(_), Ok(log)) => Source::SimulatorLog(log),
        _ => panic!(
            "exactly one of THRUST_MAIL_API_URL (a native target) and THRUST_MAIL_WRANGLER_LOG (a Worker) must be set: \
             scripts/contract.sh <target> mail starts the mail service and sets it"
        ),
    }
}

fn public_base() -> String {
    std::env::var("THRUST_PUBLIC_BASE_URL")
        .expect("THRUST_PUBLIC_BASE_URL must be set: scripts/contract.sh <target> mail sets it")
        .trim_end_matches('/')
        .to_string()
}

fn new_email() -> String {
    format!("mail-{}@example.test", uuid::Uuid::new_v4().simple())
}

async fn json_of(url: &str, query: &[(&str, String)]) -> Value {
    reqwest::Client::new().get(url).query(query).send().await.unwrap().json().await.unwrap()
}

/// The simulator's lines reach the dev server's log a few milliseconds AFTER the response that caused them (workerd prints them and wrangler
/// forwards them), so a read right after a request can miss its message, and a check that nothing was mailed would pass because the log is
/// late (measured: the first run of this pass read an empty log after a request whose message came a moment later). The log is in order,
/// so a marker settles it: ask for one more reset, for an account of its own, and wait until its message is in the log. Whatever was sent
/// before it is there too, and a read after this one is a read of everything sent so far.
async fn settle(t: &Target, log: &str) {
    let (marker, _) = sign_up(t).await;
    let asked = forgot(t, &json!({"email": marker})).await;
    assert_eq!(asked.status(), 200, "the marker request: {:?}", asked.text().await);
    for _ in 0..200 {
        if simulated_mails(log).iter().any(|m| m["To"][0]["Address"] == marker.as_str()) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the marker message never reached {log} in 10 s: either the simulator did not log it or the log is not being written");
}

/// Every message the mail service holds for `address`, in full, oldest first. A Worker's is read after a settling marker (see `settle`); a
/// native target's mail server has the message when the response is given, and needs none.
async fn mails_to(t: &Target, address: &str) -> Vec<Value> {
    match source() {
        Source::Api(api) => {
            let found = json_of(&format!("{api}/api/v1/search"), &[("query", format!("to:{address}"))]).await;
            let mut all = Vec::new();
            for m in found["messages"].as_array().cloned().unwrap_or_default().iter().rev() {
                all.push(json_of(&format!("{api}/api/v1/message/{}", m["ID"].as_str().unwrap()), &[]).await);
            }
            all
        }
        Source::SimulatorLog(log) => {
            settle(t, &log).await;
            simulated_mails(&log).into_iter().filter(|m| m["To"][0]["Address"] == address).collect()
        }
    }
}

/// `text` without its terminal colour sequences (wrangler colours the line that announces a message whatever `NO_COLOR` says).
fn without_colour(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What the simulator of the `send_email` binding wrote to the dev server's log for each message it was asked to send, oldest first, in
/// Mailpit's shape. For each one wrangler logs a line that says the binding was called and then `From:`, `To:` and `Subject:` lines, and
/// `Text:` and `HTML:` lines with the paths of the files that hold the bodies:
///
/// ```text
/// send_email binding called with MessageBuilder:
/// From: Thrust <no-reply@thrust.test>
/// To: someone@example.test
/// Subject: Reset your Thrust password
///
/// Text: /…/email-text/<id>.txt
/// ```
fn simulated_mails(log: &str) -> Vec<Value> {
    let raw = std::fs::read(log).unwrap_or_else(|e| panic!("the dev server's log, {log}, is unreadable: {e}"));
    let text = without_colour(&String::from_utf8_lossy(&raw));
    let lines: Vec<&str> = text.lines().collect();
    let mut all = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains("send_email binding called with MessageBuilder") {
            continue;
        }
        let (mut from, mut to, mut subject, mut text_file, mut html_file) = (None, None, None, None, None);
        for l in lines[i + 1..].iter().take_while(|l| !l.starts_with('[') && !l.contains("send_email binding called")) {
            if let Some(v) = l.strip_prefix("From: ") {
                from = Some(v.trim().to_string());
            } else if let Some(v) = l.strip_prefix("To: ") {
                to = Some(v.trim().to_string());
            } else if let Some(v) = l.strip_prefix("Subject: ") {
                subject = Some(v.to_string());
            } else if let Some(v) = l.strip_prefix("Text: ") {
                text_file = Some(v.trim().to_string());
            } else if let Some(v) = l.strip_prefix("HTML: ") {
                html_file = Some(v.trim().to_string());
            }
        }
        let read = |path: Option<String>| {
            path.map(|p| {
                std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("the simulator's file {p} is unreadable: {e}"))
            })
            .unwrap_or_default()
        };
        let from = from.unwrap_or_else(|| panic!("a simulated message with no From line, near line {i} of {log}"));
        // `Name <address>` or the bare address
        let (name, address) = match from.rsplit_once('<') {
            Some((name, rest)) => (name.trim().trim_matches('"').to_string(), rest.trim_end_matches('>').to_string()),
            None => (String::new(), from.clone()),
        };
        all.push(json!({
            "Subject": subject.unwrap_or_default(),
            "From": {"Address": address, "Name": name},
            "To": [{"Address": to.unwrap_or_default()}],
            "Text": read(text_file),
            "HTML": read(html_file),
        }));
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
async fn mail_the_instance_says_mail_is_on_when_the_target_has_a_mail_service_and_a_base_url() {
    let t = t();
    let body: Value = t.http.get(t.url("/api/instances/")).send().await.unwrap().json().await.unwrap();
    assert_eq!(body["config"]["is_smtp_configured"], true, "{body}");
}

/// The whole path. A person who forgot the password asks; one message reaches the mail service, which a reader that is not the app's reads for itself; its link starts with the
/// configured base and carries the account and a token; the reset page's form with them sets the new password, ends the session, and uses the
/// link up. A token one character off is refused first and changes nothing, so it is the mailed token and not any token that works.
#[tokio::test]
async fn mail_a_forgotten_password_is_reset_through_the_link_in_the_mail() {
    let t = t();
    let (email, session) = sign_up(&t).await;
    assert!(mails_to(&t, &email).await.is_empty(), "no mail before anything was asked");

    let asked = forgot(&t, &json!({"email": email})).await;
    assert_eq!(asked.status(), 200, "{:?}", asked.text().await);

    let mails = mails_to(&t, &email).await;
    assert_eq!(mails.len(), 1, "one request, one message");
    let mail = &mails[0];
    assert!(
        !mail["Text"].as_str().unwrap_or_default().is_empty(),
        "the message has a text part, read from where the mail service put it"
    );
    assert_eq!(mail["Subject"], "Reset your Thrust password");
    assert_eq!(mail["From"]["Address"], SENDER);
    assert_eq!(mail["From"]["Name"], "Thrust", "the name beside the sender");
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
    assert_eq!(mails_to(&t, &email).await.len(), 1, "the reset sent no further mail");
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
    let after_first = mails_to(&t, &email).await;
    assert_eq!(after_first.len(), 1, "the control: this request did send");
    let link = link_in(&after_first[0]);
    assert!(link.as_str().starts_with(&public_base()), "the link followed the request's Host: {link}");
    assert!(!link.as_str().contains("evil.example"), "{link}");

    // a second one, inside the cooldown, is answered the same and mails nothing
    let second = forgot(&t, &json!({"email": email})).await;
    assert_eq!(second.status(), 200, "answered as one that mailed");
    assert_eq!(second.json::<Value>().await.unwrap(), json!({"message": "Check your email to reset your password"}));
    // no wait is needed to see a message that was sent: the mail goes in the request (D-041, 5), so the mail service held it before the answer came
    assert_eq!(mails_to(&t, &email).await.len(), 1, "a request inside the cooldown mailed something");
}

// --- the sign-in with an emailed code (F-A04, D-042) ----------------------------------------------------------------------------------

/// The token of the page's form, any value, as long as the cookie and the hidden field agree: what `unique-code.tsx` gets from
/// `GET /auth/get-csrf-token/`.
const PAGE_CSRF: &str = "0123456789abcdef0123456789abcdef";

async fn ask_code(t: &Target, email: &str) -> reqwest::Response {
    t.http.post(t.url("/auth/magic-generate/")).json(&json!({ "email": email })).send().await.unwrap()
}

/// The unique-code step's form as the app posts it: urlencoded, with the hidden `csrfmiddlewaretoken` and the CSRF cookie the page holds. Without
/// them (`csrf` false) it is the form of a page that is not ours.
async fn code_form(t: &Target, email: &str, code: &str, csrf: bool) -> reqwest::Response {
    let mut pairs = vec![("email", email), ("code", code)];
    let mut req = t.no_redirect().post(t.url("/auth/magic-sign-in/"));
    if csrf {
        pairs.push(("csrfmiddlewaretoken", PAGE_CSRF));
        req = req.header("cookie", format!("{CSRF_COOKIE}={PAGE_CSRF}"));
    }
    req.form(&pairs).send().await.unwrap()
}

/// The six digits that end a code mail's subject.
fn code_in(mail: &Value) -> String {
    let subject = mail["Subject"].as_str().expect("a subject");
    let code = subject.rsplit(' ').next().unwrap_or_default();
    assert!(
        code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()),
        "a code of six digits ends the subject: {subject}"
    );
    code.to_string()
}

/// A code that is not `code`, and cannot be one that was made (a code starts at 100000).
fn wrong_code() -> &'static str {
    "000000"
}

fn refusal(code: u16, name: &str, email: &str) -> String {
    format!("/?error_code={code}&error_message={name}&email={}", email.replace('@', "%40"))
}

/// The whole path of a code. A person who would rather not type a password asks; one message reaches the mail service, which a reader that is not
/// the app's reads for itself; the code in it is the six digits of the subject and of the text, with no link; the form it is typed into signs the
/// person in with it, once, and the session works. A wrong code comes first and changes nothing, so it is the mailed code and not any code that
/// works, and a form that does not carry the page's CSRF token is refused whole.
#[tokio::test]
async fn mail_a_sign_in_code_is_mailed_and_signs_the_person_in_once() {
    let t = t();
    let (email, _) = sign_up(&t).await;
    assert!(mails_to(&t, &email).await.is_empty(), "no mail before anything was asked");

    let asked = ask_code(&t, &email).await;
    assert_eq!(asked.status(), 200);
    assert_eq!(asked.json::<Value>().await.unwrap(), json!({"key": format!("magic_{email}")}));
    let mails = mails_to(&t, &email).await;
    assert_eq!(mails.len(), 1, "one request, one message");
    let mail = &mails[0];
    let code = code_in(mail);
    assert_eq!(mail["From"]["Address"], SENDER);
    assert_eq!(mail["From"]["Name"], "Thrust", "the name beside the sender");
    assert_eq!(mail["To"].as_array().unwrap().len(), 1);
    assert_eq!(mail["To"][0]["Address"], email.as_str());
    let text = mail["Text"].as_str().expect("a text part");
    assert!(text.contains(&code), "the text carries the code: {text}");
    assert!(!text.contains("http"), "a code is typed, never followed: {text}");

    // CONTROL: a wrong code is refused and changes nothing
    let wrong = code_form(&t, &email, wrong_code(), true).await;
    assert_eq!(wrong.status(), 302);
    assert_eq!(location(&wrong), refusal(5090, "INVALID_MAGIC_CODE_SIGN_IN", &email));
    assert!(SetCookie::all(&wrong).is_empty(), "a refusal sets no cookie");
    // a form without the page's token is refused whole, and spends no try
    let outsider = code_form(&t, &email, &code, false).await;
    assert_eq!(outsider.status(), 403, "{:?}", outsider.text().await);

    // the code as mailed, with spaces round it as a paste may have them
    let ok = code_form(&t, &email, &format!(" {code} "), true).await;
    assert_eq!(ok.status(), 302);
    assert_eq!(location(&ok), "/");
    let cookies = SetCookie::all(&ok);
    let session = SetCookie::find(&cookies, SESSION_COOKIE).expect("a sign-in by code makes a session").value.clone();
    assert_eq!(t.me_status(&session).await, 200, "the session works");
    assert_eq!(sign_in_status(&t, &email, STRONG).await, 200, "the password is as it was");

    // used up
    let again = code_form(&t, &email, &code, true).await;
    assert_eq!(location(&again), refusal(5095, "EXPIRED_MAGIC_CODE_SIGN_IN", &email));
    assert!(SetCookie::all(&again).is_empty());
    assert_eq!(mails_to(&t, &email).await.len(), 1, "the sign-in sent no further mail");
}

/// What is mailed, and how many: an address nobody has is answered as a sent one and mailed nothing, and one that is not an address is a 400; an
/// address is made four codes in a window and the fifth is refused with the pin's 5100, the four having been mailed one each; and a new code
/// replaces the old, so that of four mailed codes only the newest signs in. Each "nothing" is beside a request that did send.
#[tokio::test]
async fn mail_what_a_code_is_made_for_and_how_many_are() {
    let t = t();
    let (email, _) = sign_up(&t).await;

    let stranger = new_email();
    let r = ask_code(&t, &stranger).await;
    assert_eq!(r.status(), 200, "answered as a sent one");
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"key": format!("magic_{stranger}")}));
    let bad = ask_code(&t, "not an address").await;
    assert_eq!(bad.status(), 400);
    assert_eq!(bad.json::<Value>().await.unwrap()["error_code"], 5005);

    for n in 1..=4 {
        assert_eq!(ask_code(&t, &email).await.status(), 200, "request {n}");
    }
    let fifth = ask_code(&t, &email).await;
    assert_eq!(fifth.status(), 400);
    let v: Value = fifth.json().await.unwrap();
    assert_eq!(v["error_code"], 5100);
    assert_eq!(v["error_message"], "EMAIL_CODE_ATTEMPT_EXHAUSTED_SIGN_IN");
    assert_eq!(v["email"], email.as_str());

    let mails = mails_to(&t, &email).await;
    assert_eq!(mails.len(), 4, "the control: four requests sent, and the fifth sent nothing");
    assert!(mails_to(&t, &stranger).await.is_empty(), "a stranger was mailed a code");

    // oldest first: three replaced codes, and the newest
    let codes: Vec<String> = mails.iter().map(code_in).collect();
    for (n, old) in codes[..3].iter().enumerate() {
        if *old == codes[3] {
            continue; // the same six digits drawn twice: it is the newest, and works
        }
        let r = code_form(&t, &email, old, true).await;
        assert_eq!(location(&r), refusal(5090, "INVALID_MAGIC_CODE_SIGN_IN", &email), "code {} was replaced", n + 1);
    }
    let ok = code_form(&t, &email, &codes[3], true).await;
    assert_eq!(location(&ok), "/", "the newest signs in");
}

/// The tries: a code allows five, the fifth wrong one ends it (5100), the right code after that is 5095, and a person who asks again is mailed
/// a new code that works. The count of tries is the code's, not the person's.
#[tokio::test]
async fn mail_five_wrong_tries_end_a_code_and_a_new_one_works() {
    let t = t();
    let (email, _) = sign_up(&t).await;
    assert_eq!(ask_code(&t, &email).await.status(), 200);
    let first = code_in(&mails_to(&t, &email).await[0]);
    for n in 1..=4 {
        let r = code_form(&t, &email, wrong_code(), true).await;
        assert_eq!(location(&r), refusal(5090, "INVALID_MAGIC_CODE_SIGN_IN", &email), "try {n}");
    }
    let last = code_form(&t, &email, wrong_code(), true).await;
    assert_eq!(location(&last), refusal(5100, "EMAIL_CODE_ATTEMPT_EXHAUSTED_SIGN_IN", &email));
    let after = code_form(&t, &email, &first, true).await;
    assert_eq!(
        location(&after),
        refusal(5095, "EXPIRED_MAGIC_CODE_SIGN_IN", &email),
        "the right code, after it was ended"
    );
    assert!(SetCookie::all(&after).is_empty());

    assert_eq!(ask_code(&t, &email).await.status(), 200, "the window still allows another");
    let mails = mails_to(&t, &email).await;
    assert_eq!(mails.len(), 2);
    let second = code_in(&mails[1]);
    let ok = code_form(&t, &email, &second, true).await;
    assert_eq!(location(&ok), "/", "the new code works");
}

/// A code that is right for an account that was deactivated after it asked is refused with the pin's 5019 and used up, and no session is made.
#[tokio::test]
async fn mail_a_right_code_of_a_deactivated_account_is_refused_and_used_up() {
    let t = t();
    let (email, session) = sign_up(&t).await;
    assert_eq!(ask_code(&t, &email).await.status(), 200);
    let code = code_in(&mails_to(&t, &email).await[0]);
    let me: Value = t.me(|r| r.bearer_auth(&session)).await.json().await.unwrap();
    let user = UserId(me["id"].as_str().unwrap().parse().unwrap());
    t.set_user_active(user, false).await;

    let refused = code_form(&t, &email, &code, true).await;
    assert_eq!(refused.status(), 302);
    assert_eq!(location(&refused), refusal(5019, "USER_ACCOUNT_DEACTIVATED", &email));
    assert!(SetCookie::all(&refused).is_empty(), "no session for a deactivated account");
    t.set_user_active(user, true).await;
    let used = code_form(&t, &email, &code, true).await;
    assert_eq!(
        location(&used),
        refusal(5095, "EXPIRED_MAGIC_CODE_SIGN_IN", &email),
        "the right code was used up by the refusal"
    );
}
