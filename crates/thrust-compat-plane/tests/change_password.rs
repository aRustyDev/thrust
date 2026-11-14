//! `POST /auth/change-password/` (X09, D-039): what Profile -> Security calls. The pin answers JSON; the recorded row
//! (`parity_x09_change_password`) shows its side. These tests state what the recording cannot: every combination of what the old
//! and the new password can be, the shapes of a body that no client sends, what a caller who holds no session gets, and the
//! allowance that Thrust adds and the pin does not have.
//!
//! What must hold: the answer to a body is a function of it alone, in the order the pin checks (an old password that is nothing, a
//! new one that is nothing, an old one that is wrong, a new one that is weak); a refusal changes nothing and sets no cookie; a
//! change swaps the password, ends the session that asked, and sets a new one in the same answer; a body that is not a JSON object
//! of text is an empty one; a caller with no live session is a 401 and changes nothing; and the old password's guesses are
//! counted with the account's sign-in guesses, in both directions.

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

const A: &str = "changes@example.test";
const B: &str = "bystander@example.test";
const NEW: &str = "Another-Sample-Pa55word-Yak-27";
const WEAK: &str = "password123";
const CHANGE: &str = "/auth/change-password/";
const SIGN_IN: &str = "/auth/sign-in/";

fn change(router: &axum::Router, jar: &Jar, who: &str, body: Value) -> Value {
    call(router, jar, who, "POST", CHANGE, body)
}

fn me_status(router: &axum::Router, jar: &Jar, who: &str) -> u64 {
    call(router, jar, who, "GET", "/api/users/me/", Value::Null)["status"].as_u64().expect("a status")
}

fn sign_in_status(router: &axum::Router, email: &str, password: &str) -> u64 {
    call(router, &Jar::new(), "", "POST", SIGN_IN, json!({"email": email, "password": password}))["status"]
        .as_u64()
        .expect("a status")
}

#[derive(Debug, Clone, Copy)]
enum OldIs {
    Absent,
    Null,
    Empty,
    Zero,
    False,
    EmptyList,
    EmptyObject,
    Wrong,
    Right,
    Number,
    List,
}

#[derive(Debug, Clone, Copy)]
enum NewIs {
    Absent,
    Null,
    Empty,
    Strong,
    Weak,
    Number,
    List,
}

/// The body with `old_password` and `new_password` as given; an absent one is not a member.
fn body_of(old: OldIs, new: NewIs) -> Value {
    let mut body = serde_json::Map::new();
    let old_value = match old {
        OldIs::Absent => None,
        OldIs::Null => Some(Value::Null),
        OldIs::Empty => Some(json!("")),
        OldIs::Zero => Some(json!(0)),
        OldIs::False => Some(json!(false)),
        OldIs::EmptyList => Some(json!([])),
        OldIs::EmptyObject => Some(json!({})),
        OldIs::Wrong => Some(json!("not the password at all")),
        OldIs::Right => Some(json!(STRONG)),
        OldIs::Number => Some(json!(12345)),
        OldIs::List => Some(json!([STRONG])),
    };
    let new_value = match new {
        NewIs::Absent => None,
        NewIs::Null => Some(Value::Null),
        NewIs::Empty => Some(json!("")),
        NewIs::Strong => Some(json!(NEW)),
        NewIs::Weak => Some(json!(WEAK)),
        NewIs::Number => Some(json!(123_456_789)),
        NewIs::List => Some(json!([NEW])),
    };
    if let Some(v) = old_value {
        body.insert("old_password".into(), v);
    }
    if let Some(v) = new_value {
        body.insert("new_password".into(), v);
    }
    Value::Object(body)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    MissingOld,
    MissingNew,
    WrongOld,
    TooWeak,
    Changed,
}

/// What the pin answers, from the recording and the source (`ChangePasswordEndpoint`): written out from Python's rules, not from this
/// crate's handler. A value is nothing when it is Python-falsy; the old password is judged before the new one is looked at for
/// strength; a `new_password` that is not text is a 500 there and a weak password here (D-039), said where the strength is.
fn expected(old: OldIs, new: NewIs) -> Answer {
    let old_is_nothing = matches!(
        old,
        OldIs::Absent | OldIs::Null | OldIs::Empty | OldIs::Zero | OldIs::False | OldIs::EmptyList | OldIs::EmptyObject
    );
    let new_is_nothing = matches!(new, NewIs::Absent | NewIs::Null | NewIs::Empty);
    if old_is_nothing {
        Answer::MissingOld
    } else if new_is_nothing {
        Answer::MissingNew
    } else if !matches!(old, OldIs::Right) {
        Answer::WrongOld
    } else if !matches!(new, NewIs::Strong) {
        Answer::TooWeak
    } else {
        Answer::Changed
    }
}

fn answer_body(a: Answer) -> Value {
    match a {
        Answer::MissingOld => {
            json!({"error_code": 5138, "error_message": "MISSING_PASSWORD", "error": "Old password is missing"})
        }
        Answer::MissingNew => {
            json!({"error_code": 5138, "error_message": "MISSING_PASSWORD", "error": "Old or new password is missing"})
        }
        Answer::WrongOld => {
            json!({"error_code": 5135, "error_message": "INCORRECT_OLD_PASSWORD", "error": "Old password is not correct"})
        }
        Answer::TooWeak => json!({"error_code": 5021, "error_message": "PASSWORD_TOO_WEAK"}),
        Answer::Changed => json!({"message": "Password updated successfully"}),
    }
}

/// Every pair of what the old and the new password can be, answered as `expected` says; a refusal sets no cookie and changes no
/// password or session (the other account's too); a change sets both cookies, ends the session that asked, and the one it set is
/// live, and the new password signs in where the old one no longer does. 11 x 7 = 77 cases, the whole domain.
#[rstest]
fn the_answer_is_a_function_of_the_two_fields_and_a_refusal_changes_nothing(
    #[values(
        OldIs::Absent,
        OldIs::Null,
        OldIs::Empty,
        OldIs::Zero,
        OldIs::False,
        OldIs::EmptyList,
        OldIs::EmptyObject,
        OldIs::Wrong,
        OldIs::Right,
        OldIs::Number,
        OldIs::List
    )]
    old: OldIs,
    #[values(NewIs::Absent, NewIs::Null, NewIs::Empty, NewIs::Strong, NewIs::Weak, NewIs::Number, NewIs::List)]
    new: NewIs,
) {
    let (router, _db, mut jar) = world(&[A, B]);
    let want = expected(old, new);
    let raw = send(
        &router,
        &json!("change"),
        &json!({"method": "POST", "path": CHANGE, "as": A, "body": body_of(old, new)}),
        &jar,
    );
    let got = &raw["response"];
    let case = format!("{old:?} {new:?}");
    assert_eq!(got["status"], if want == Answer::Changed { 200 } else { 400 }, "{case}: {got}");
    assert_eq!(got["body"], answer_body(want), "{case}");

    if want == Answer::Changed {
        let made =
            session_token(&raw).unwrap_or_else(|| panic!("{case}: a change sets the caller's new session: {raw}"));
        assert!(both_cookies_set(&raw), "{case}: and its CSRF cookie");
        assert_eq!(me_status(&router, &jar, A), 401, "{case}: the session that asked is over");
        jar.insert(jar_key(A), made);
        assert_eq!(me_status(&router, &jar, A), 200, "{case}: the session it set is live");
        assert_eq!(sign_in_status(&router, A, STRONG), 400, "{case}: the old password no longer signs in");
        assert_eq!(sign_in_status(&router, A, NEW), 200, "{case}: the new one does");
    } else {
        assert!(raw["response"]["headers"].get("set-cookie").is_none(), "{case}: a refusal sets no cookie");
        assert_eq!(me_status(&router, &jar, A), 200, "{case}: the session stands");
        assert_eq!(sign_in_status(&router, A, STRONG), 200, "{case}: the password is the old one");
        assert_eq!(sign_in_status(&router, A, NEW), 400, "{case}");
    }
    assert_eq!(me_status(&router, &jar, B), 200, "{case}: another account's session is not touched");
    assert_eq!(sign_in_status(&router, B, STRONG), 200, "{case}: nor its password");
}

fn both_cookies_set(raw: &Value) -> bool {
    let all: Vec<&str> = match &raw["response"]["headers"]["set-cookie"] {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    ["__Host-thrust_session", "__Host-thrust_csrf"].iter().all(|name| {
        all.iter().any(|c| {
            c.strip_prefix(&format!("{name}=")).is_some_and(|rest| !rest.starts_with(';') && !c.contains("Max-Age=0"))
        })
    }) && raw["response"]["headers"]["cache-control"] == "no-store"
}

/// A body that is no JSON object of text (an array, a string, a number, `null`, nothing, text that is not JSON, a form post) is an
/// empty one, as it is to the pin's sign-in: "Old password is missing". The pin answers a 500 page for the arrays, strings and
/// numbers and takes a form post as it is (D-039); no client of Thrust sends any of them.
#[rstest]
fn a_body_that_is_no_json_object_is_an_empty_one(
    #[values("[]", r#""text""#, "7", "null", "", "not json", "old_password=x&new_password=y", r#"["old_password"]"#)]
    raw_body: &str,
) {
    let (router, _db, jar) = world(&[A]);
    let r = call_raw(&router, &jar, A, "POST", CHANGE, raw_body);
    assert_eq!(r["status"], 400, "{raw_body:?}: {r}");
    assert_eq!(r["body"], answer_body(Answer::MissingOld), "{raw_body:?}");
    assert_eq!(sign_in_status(&router, A, STRONG), 200, "{raw_body:?}: nothing changed");
}

#[derive(Debug, Clone, Copy)]
enum Held {
    Nothing,
    AnUnknownCookie,
    ASessionThatHasEnded,
}

/// A caller with no live session is the 401 of every other signed-in route (the body the pin's DRF gives), and changes nothing and
/// sets no cookie, whatever the body: the right one included.
#[rstest]
fn a_caller_with_no_live_session_is_a_401_and_changes_nothing(
    #[values(Held::Nothing, Held::AnUnknownCookie, Held::ASessionThatHasEnded)] held: Held,
) {
    let (router, _db, mut jar) = world(&[A]);
    let who = match held {
        Held::Nothing => "",
        Held::AnUnknownCookie => {
            jar.insert(jar_key("ghost@example.test"), "not-a-session-token".to_string());
            "ghost@example.test"
        }
        Held::ASessionThatHasEnded => {
            assert_eq!(call(&router, &jar, A, "POST", "/auth/sign-out/", Value::Null)["status"], 200);
            A
        }
    };
    let r = change(&router, &jar, who, json!({"old_password": STRONG, "new_password": NEW}));
    assert_eq!(r["status"], 401, "{held:?}: {r}");
    assert_eq!(r["body"], json!({"detail": "Authentication credentials were not provided."}), "{held:?}");
    assert_eq!(sign_in_status(&router, A, STRONG), 200, "{held:?}: nothing changed");
    assert_eq!(sign_in_status(&router, A, NEW), 400, "{held:?}");
}

/// The old password's guesses and the account's sign-in guesses are one allowance (10 in a window by default), counted before any
/// hashing: after ten wrong guesses by either door the eleventh is a 429 with the pin's code for it and a `Retry-After`, from the
/// other door too, and the right password does not get past it.
#[rstest]
fn the_old_password_and_sign_in_share_one_allowance(#[values(true, false)] sign_ins_first: bool) {
    let (router, _db, jar) = world(&[A]);
    for n in 0..10 {
        if sign_ins_first {
            assert_eq!(sign_in_status(&router, A, "wrong guess"), 400, "sign-in guess {n}");
        } else {
            let r = change(&router, &jar, A, json!({"old_password": "wrong guess", "new_password": NEW}));
            assert_eq!(r["status"], 400, "change guess {n}: {r}");
            assert_eq!(r["body"], answer_body(Answer::WrongOld), "change guess {n}");
        }
    }
    if sign_ins_first {
        let r = change(&router, &jar, A, json!({"old_password": STRONG, "new_password": NEW}));
        assert_eq!(r["status"], 429, "{r}");
        assert_eq!(r["body"], json!({"error_code": 5900, "error_message": "RATE_LIMIT_EXCEEDED"}));
        assert!(r["headers"]["retry-after"].as_str().is_some_and(|s| s.parse::<u64>().is_ok_and(|n| n > 0)), "{r}");
        assert_eq!(me_status(&router, &jar, A), 200, "nothing changed: the session stands");
    } else {
        assert_eq!(sign_in_status(&router, A, STRONG), 429, "the right password is not heard past the allowance");
        let r = change(&router, &jar, A, json!({"old_password": STRONG, "new_password": NEW}));
        assert_eq!(r["status"], 429, "{r}");
    }
}
