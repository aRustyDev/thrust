//! What the user routes derive from what was stored, which a recording cannot show because the journey never sets it: the
//! avatar and cover image URLs of the short user, and that a patch of one thing does not undo another.
//!
//! What must hold: `avatar_url` is the avatar when there is one and null when there is not, `cover_image_url` the cover
//! image likewise; the short user (S02) and the long one (S10) agree on the fields they share; and the user is the
//! caller's own, never another account's.

mod common;

use axum::Router;
use common::*;
use proptest::prelude::*;
use serde_json::{json, Value};

const STRONG: &str = "Users-Sample-Pa55word-Zebra-81";

/// A world with a signed-up account, and the jar that holds its session.
fn world_with(email: &str) -> (Router, Jar) {
    let router = app();
    let mut jar = Jar::new();
    let raw = send(&router, &json!("sign-up"), &json!({"method": "POST", "path": "/auth/sign-up/", "body": {"email": email, "password": STRONG}}), &jar);
    assert_eq!(raw["response"]["status"], 200, "{raw}");
    jar.insert(jar_key(email), session_token(&raw).expect("a session"));
    (router, jar)
}

fn call(router: &Router, jar: &Jar, who: &str, method: &str, path: &str, body: Value) -> Value {
    let mut req = json!({"method": method, "path": path, "as": who});
    if !body.is_null() {
        req["body"] = body;
    }
    send(router, &json!("call"), &req, jar)["response"].clone()
}

fn me(router: &Router, jar: &Jar, who: &str) -> Value {
    let r = call(router, jar, who, "GET", "/api/users/me/", Value::Null);
    assert_eq!(r["status"], 200, "{r}");
    r["body"].clone()
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// The URLs of the short user are the avatar and the cover image as stored: present when set, null when empty or cleared.
    #[test]
    fn prop_the_short_users_urls_follow_the_avatar_and_the_cover_image(
        avatar in prop::option::of("[a-z0-9./:-]{1,30}"),
        cover in prop::option::of("[a-z0-9]{1,12}"),
    ) {
        let (router, jar) = world_with("urls@example.test");
        let who = "urls@example.test";
        if let Some(a) = &avatar {
            let r = call(&router, &jar, who, "PATCH", "/api/users/me/", json!({"avatar": a}));
            prop_assert_eq!(&r["status"], 200);
        }
        if let Some(c) = &cover {
            let r = call(&router, &jar, who, "PATCH", "/api/users/me/", json!({"cover_image": format!("https://example.test/{c}")}));
            prop_assert_eq!(&r["status"], 200, "{}", r);
        }
        let user = me(&router, &jar, who);
        // an avatar of only whitespace is trimmed to nothing, and then there is none
        let stored_avatar = avatar.as_deref().map(str::trim).unwrap_or_default();
        prop_assert_eq!(user["avatar"].as_str(), Some(stored_avatar));
        prop_assert_eq!(&user["avatar_url"], &if stored_avatar.is_empty() { Value::Null } else { json!(stored_avatar) });
        let want_cover = cover.as_ref().map(|c| format!("https://example.test/{c}"));
        prop_assert_eq!(user["cover_image"].as_str(), want_cover.as_deref());
        prop_assert_eq!(user["cover_image_url"].as_str(), want_cover.as_deref());
    }
}

#[test]
fn clearing_the_cover_image_clears_its_url_and_the_avatar_survives_it() {
    let (router, jar) = world_with("clear@example.test");
    let who = "clear@example.test";
    call(&router, &jar, who, "PATCH", "/api/users/me/", json!({"avatar": "a.png", "cover_image": "https://example.test/c"}));
    let r = call(&router, &jar, who, "PATCH", "/api/users/me/", json!({"cover_image": null}));
    assert_eq!(r["status"], 200);
    let user = me(&router, &jar, who);
    assert_eq!((user["cover_image"].clone(), user["cover_image_url"].clone()), (Value::Null, Value::Null));
    assert_eq!(user["avatar_url"], "a.png", "an unrelated field survives");
}

#[test]
fn the_short_user_and_the_long_user_agree_on_what_they_share() {
    let (router, jar) = world_with("share@example.test");
    let who = "share@example.test";
    let long = call(&router, &jar, who, "PATCH", "/api/users/me/", json!({"first_name": "Sam", "last_name": "Q", "user_timezone": "Asia/Kolkata"}))["body"].clone();
    let short = me(&router, &jar, who);
    for key in short.as_object().unwrap().keys() {
        if let Some(l) = long.get(key) {
            assert_eq!(&short[key], l, "`{key}` differs between GET (short) and PATCH (long)");
        }
    }
    // and the long one does carry the ones the short one lacks, the short one the two URLs the long one lacks
    assert!(long.get("token").is_some() && short.get("token").is_none());
    assert!(short.get("avatar_url").is_some() && long.get("avatar_url").is_none());
}

#[test]
fn a_user_is_the_callers_own() {
    let (router, mut jar) = world_with("one@example.test");
    // a second account in the same world
    let raw = send(&router, &json!("sign-up"), &json!({"method": "POST", "path": "/auth/sign-up/", "body": {"email": "two@example.test", "password": STRONG}}), &jar);
    jar.insert(jar_key("two@example.test"), session_token(&raw).unwrap());
    call(&router, &jar, "one@example.test", "PATCH", "/api/users/me/", json!({"first_name": "One"}));
    call(&router, &jar, "one@example.test", "PATCH", "/api/users/me/profile/", json!({"role": "designer"}));
    assert_eq!(me(&router, &jar, "one@example.test")["first_name"], "One");
    assert_eq!(me(&router, &jar, "two@example.test")["first_name"], "", "the other account is untouched");
    assert_eq!(me(&router, &jar, "two@example.test")["email"], "two@example.test");
    let profile = call(&router, &jar, "two@example.test", "GET", "/api/users/me/profile/", Value::Null);
    assert!(profile["body"]["role"].is_null(), "and so is its profile");
}
