//! The upstream request and the response the client gets (src/forward.rs, research/0016).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions, not the helpers beside them"
)]
mod common;

use common::proptest_config;
use proptest::prelude::*;
use rstest::rstest;
use thrust_edge::forward::{self, ROUTE_HEADER};

const PREVIEW: &str = "v0-3-0-thrust.team.workers.dev";
const SHARED: &str = "https://thrust.sow.foo";

proptest! {
    #![proptest_config(proptest_config(512))]

    /// The upstream URL keeps the target's host whatever the client's path is (`//evil`, `/@evil`, `/..`): the host is the
    /// part of the URL between `https://` and the first `/`, and it is the base's.
    #[test]
    fn prop_the_upstream_host_is_never_the_clients(path in "/[ -~]{0,60}") {
        let url = forward::upstream_url(&format!("https://{PREVIEW}"), &path).unwrap();
        let after = url.strip_prefix("https://").unwrap();
        let host = after.split('/').next().unwrap();
        prop_assert_eq!(host, PREVIEW);
        prop_assert_eq!(&after[PREVIEW.len()..], path.as_str());
    }

    /// Every Set-Cookie for CF_Authorization (any case) is removed, every other header stays in order, and the route header
    /// is there exactly once with the edge's value, whatever the upstream sent.
    #[test]
    fn prop_the_response_headers(
        headers in prop::collection::vec(prop_oneof![
            ("[A-Za-z][A-Za-z0-9-]{0,12}", "[ -~]{0,20}"),
            (Just("Set-Cookie".to_string()), "[A-Za-z][A-Za-z0-9_]{0,10}=[A-Za-z0-9]{0,8}(; Path=/)?"),
            (Just("set-cookie".to_string()), "(CF_Authorization|cf_authorization|CF_AUTHORIZATION)=[A-Za-z0-9.]{1,20}; Path=/"),
            (Just("X-Thrust-Route".to_string()), "[a-z]{1,8}"),
        ], 0..10),
        route in "[a-z0-9.-]{1,12}",
    ) {
        let out = forward::response_headers(&headers, &route, None, SHARED);
        let want: Vec<(String, String)> = headers.iter().filter(|(n, v)| {
            let l = n.to_ascii_lowercase();
            l != ROUTE_HEADER && !(l == "set-cookie" && v.split('=').next().unwrap().trim().eq_ignore_ascii_case("cf_authorization"))
        }).cloned().chain(std::iter::once((ROUTE_HEADER.to_string(), route.clone()))).collect();
        prop_assert_eq!(out, want);
    }
}

/// A Location naming the Preview host, absolute or scheme-relative, in any case, goes to the shared origin with its path,
/// query and fragment; anything else is left alone, including a host that merely starts with the Preview's.
#[rstest]
#[case::absolute("https://v0-3-0-thrust.team.workers.dev/landed", "https://thrust.sow.foo/landed")]
#[case::bare_host("https://v0-3-0-thrust.team.workers.dev", "https://thrust.sow.foo")]
#[case::query("https://v0-3-0-thrust.team.workers.dev?next=/x", "https://thrust.sow.foo?next=/x")]
#[case::fragment("https://v0-3-0-thrust.team.workers.dev#top", "https://thrust.sow.foo#top")]
#[case::http("http://v0-3-0-thrust.team.workers.dev/a", "https://thrust.sow.foo/a")]
#[case::scheme_relative("//v0-3-0-thrust.team.workers.dev/a", "https://thrust.sow.foo/a")]
#[case::upper_case_host("https://V0-3-0-THRUST.team.workers.dev/a", "https://thrust.sow.foo/a")]
#[case::spec_spike_redirect("https://v0-3-0-thrust.team.workers.dev/landed", "https://thrust.sow.foo/landed")]
#[case::relative("/landed", "/landed")]
#[case::other_host("https://auth.example.org/authorize?x=1", "https://auth.example.org/authorize?x=1")]
#[case::host_prefix(
    "https://v0-3-0-thrust.team.workers.dev.evil.example/a",
    "https://v0-3-0-thrust.team.workers.dev.evil.example/a"
)]
#[case::host_with_port("https://v0-3-0-thrust.team.workers.dev:443/a", "https://v0-3-0-thrust.team.workers.dev:443/a")]
fn locations(#[case] location: &str, #[case] want: &str) {
    assert_eq!(forward::rewrite_location(location, PREVIEW, SHARED), want);
}

#[test]
fn a_location_is_rewritten_only_on_a_targets_response() {
    let h = vec![("Location".to_string(), format!("https://{PREVIEW}/x"))];
    assert_eq!(forward::response_headers(&h, "v0.3.0", Some(PREVIEW), SHARED)[0].1, "https://thrust.sow.foo/x");
    assert_eq!(forward::response_headers(&h, "prod", None, SHARED)[0].1, format!("https://{PREVIEW}/x"));
}

/// The spike's finding: among several cookies the Preview sets, Access's goes and the application's stay.
#[test]
fn spec_access_cookie_is_removed_among_several() {
    let h = vec![
        ("set-cookie".to_string(), "__Host-thrust_session=s; Path=/; Secure; HttpOnly; SameSite=Lax".to_string()),
        (
            "set-cookie".to_string(),
            "CF_Authorization=eyJ.x.y; Expires=Mon, 16 Nov 2026 05:15:38 GMT; Path=/; Secure; HttpOnly; SameSite=none"
                .to_string(),
        ),
        ("set-cookie".to_string(), "__Host-thrust_csrf=c; Path=/; Secure; SameSite=Lax".to_string()),
    ];
    let out = forward::response_headers(&h, "v0.3.0", Some(PREVIEW), SHARED);
    let cookies: Vec<&str> = out.iter().filter(|(n, _)| n == "set-cookie").map(|(_, v)| v.as_str()).collect();
    assert_eq!(cookies.len(), 2);
    assert!(cookies.iter().all(|c| !c.starts_with("CF_Authorization")));
}

#[test]
fn the_upstream_headers_drop_host_and_carry_the_shared_host_once() {
    let scrubbed = vec![
        ("Host".to_string(), "thrust.sow.foo".to_string()),
        ("Accept".to_string(), "*/*".to_string()),
        ("x-forwarded-host".to_string(), "evil.example".to_string()),
    ];
    let out = forward::upstream_headers(&scrubbed, "thrust.sow.foo");
    assert!(!out.iter().any(|(n, _)| n.eq_ignore_ascii_case("host")));
    let xfh: Vec<&str> =
        out.iter().filter(|(n, _)| n.eq_ignore_ascii_case("x-forwarded-host")).map(|(_, v)| v.as_str()).collect();
    assert_eq!(xfh, ["thrust.sow.foo"]);
    assert!(out.contains(&("x-forwarded-proto".to_string(), "https".to_string())));
    assert!(out.contains(&("Accept".to_string(), "*/*".to_string())));
}

#[rstest]
#[case::http_base("http://x-thrust.team.workers.dev", "/a")]
#[case::trailing_slash("https://x-thrust.team.workers.dev/", "/a")]
#[case::path_without_slash("https://x-thrust.team.workers.dev", "a")]
#[case::empty_path("https://x-thrust.team.workers.dev", "")]
fn an_upstream_url_that_would_be_wrong_is_none(#[case] base: &str, #[case] path: &str) {
    assert_eq!(forward::upstream_url(base, path), None);
}
