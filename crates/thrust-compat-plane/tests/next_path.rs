//! Where the browser goes after a sign-up (and a sign-in, which shares the code): the app's root, with `?next_path=` when
//! the client asked for a place and the place is safe. The pinned Plane's fixtures only show a safe path, so the open
//! redirect guard (a second slash, a scheme, a traversal, a backslash, a control character, a script pattern, a path that
//! is too long) is tested here, over generated paths, through the router.
//!
//! What must hold: a safe path is kept, byte for byte, in the query; a path that is not one slash and then a path is
//! dropped, whatever it holds; one forbidden thing anywhere in an otherwise safe path drops it; characters that would
//! corrupt the query are percent-encoded (Plane writes them raw, which is a bug this does not copy).

mod common;

use common::*;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Value};

const STRONG: &str = "NextPath-Sample-Pa55word-Zebra-81";

/// The `redirect_path` a sign-up with this `next_path` is answered with, on a fresh world.
fn redirect_for(next_path: &str) -> String {
    let router = app();
    let req = json!({
        "method": "POST",
        "path": "/auth/sign-up/",
        "body": {"email": "next@example.test", "password": STRONG, "next_path": next_path},
    });
    let raw = send(&router, &json!("sign-up"), &req);
    assert_eq!(raw["response"]["status"], 200, "{raw}");
    let body: &Value = &raw["response"]["body"];
    body["redirect_path"].as_str().unwrap_or_else(|| panic!("{body}")).to_string()
}

fn segment() -> impl Strategy<Value = String> {
    "[a-z0-9_-][a-z0-9_.-]{0,7}".prop_filter("no traversal", |s| !s.contains(".."))
}

/// `/` and then one to four segments: a safe path.
fn safe_path() -> impl Strategy<Value = String> {
    prop::collection::vec(segment(), 1..5).prop_map(|segs| format!("/{}", segs.join("/")))
}

proptest! {
    #![proptest_config(proptest_config(128))]

    #[test]
    fn prop_a_safe_path_is_kept_in_the_query(p in safe_path()) {
        prop_assert_eq!(redirect_for(&p), format!("/?next_path={p}"));
    }

    /// Anything that is not one slash and then a path is dropped: no leading slash, or two.
    #[test]
    fn prop_a_path_without_a_single_leading_slash_is_dropped(
        s in "[a-zA-Z0-9:/._-]{0,40}".prop_filter("not a safe start", |s| !(s.starts_with('/') && !s.starts_with("//"))),
    ) {
        prop_assert_eq!(redirect_for(&s), "/");
    }

    /// Control for the property that keeps a path: one forbidden thing put anywhere into a safe path drops it.
    #[test]
    fn prop_one_forbidden_thing_anywhere_drops_an_otherwise_safe_path(
        p in safe_path(),
        at in 0usize..60,
        bad in prop::sample::select(vec!["\\", "\n", "\t", "\u{7f}", "..", "javascript:", "DATA:", "<script", "%2e%2e", "onload="]),
    ) {
        let mut chars: Vec<char> = p.chars().collect();
        // never before the leading slash: the point is a bad thing inside a path that starts well
        let at = 1 + at % chars.len();
        for (i, c) in bad.chars().enumerate() {
            chars.insert(at + i, c);
        }
        let broken: String = chars.into_iter().collect();
        prop_assert_eq!(redirect_for(&broken), "/", "{:?}", broken);
    }
}

/// Rule 8 for the properties above: sampled with a fixed seed at their own case count, the generators must reach the
/// inputs that matter, or a guard that fails on them would pass unseen.
#[test]
fn prop_next_path_strategies_reach_every_input_class() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let (mut one_segment, mut four_segments, mut dotted, mut dashed) = (false, false, false, false);
    for _ in 0..128 {
        let p = safe_path().new_tree(&mut runner).unwrap().current();
        let segs = p.matches('/').count();
        one_segment |= segs == 1;
        four_segments |= segs == 4;
        dotted |= p.contains('.');
        dashed |= p.contains('-');
    }
    assert!(one_segment && four_segments && dotted && dashed, "[one segment, four, dotted, dashed]: {:?}", [one_segment, four_segments, dotted, dashed]);
}

/// Spec examples from the contract the stock app relies on: characters that would end or corrupt the query are encoded,
/// and the slash stays a slash (Python's `quote(s, safe="/")`, which is how Plane's own client would read it back).
#[rstest]
#[case::space("/a b", "/?next_path=/a%20b")]
#[case::query("/a?b=c&d=e", "/?next_path=/a%3Fb%3Dc%26d%3De")]
#[case::fragment("/a#b", "/?next_path=/a%23b")]
#[case::percent("/a%20b", "/?next_path=/a%2520b")]
#[case::unicode("/caf\u{e9}", "/?next_path=/caf%C3%A9")]
#[case::root("/", "/?next_path=/")]
fn a_path_with_characters_that_would_corrupt_the_query_is_encoded(#[case] given: &str, #[case] want: &str) {
    assert_eq!(redirect_for(given), want);
}

#[test]
fn the_length_limit_is_exact() {
    let at = |n: usize| format!("/{}", "a".repeat(n - 1));
    assert_eq!(redirect_for(&at(500)), format!("/?next_path={}", at(500)), "500 is the longest");
    assert_eq!(redirect_for(&at(501)), "/", "501 is too long");
}

#[test]
fn no_next_path_and_an_empty_one_both_go_to_the_root() {
    // a fresh world per call: the same email signs up once
    let go = |body: Value| {
        let raw = send(&app(), &json!("n"), &json!({"method": "POST", "path": "/auth/sign-up/", "body": body}));
        raw["response"]["body"]["redirect_path"].as_str().unwrap().to_string()
    };
    assert_eq!(go(json!({"email": "a@example.test", "password": STRONG})), "/");
    assert_eq!(go(json!({"email": "a@example.test", "password": STRONG, "next_path": ""})), "/");
    assert_eq!(go(json!({"email": "a@example.test", "password": STRONG, "next_path": null})), "/");
}
