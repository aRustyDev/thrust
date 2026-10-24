//! `/api/instances/` and the three base URLs it carries. The parity fixture cannot check them: they name the deployment,
//! so the normaliser replaces them with `<base_url>` on both sides. Thrust serves the app and the API from one origin,
//! so each of the three is the origin the browser used: the scheme the proxy said (`x-forwarded-proto`, trusted for the
//! scheme only) and the `Host` it asked for.
//!
//! What must hold: all three are that origin; a scheme other than `http` or `https` is not trusted and falls back to
//! `http`; with a chain of proxies the first scheme counts; with no `Host` it is `localhost`.

mod common;

use axum::body::Body;
use axum::http::Request;
use common::*;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::Value;
use tower::ServiceExt;

/// The three base URLs `/api/instances/` answers for a request with these headers, on a fresh world.
fn base_urls(host: Option<&str>, proto: Option<&str>) -> [String; 3] {
    let mut req = Request::builder().uri("/api/instances/");
    if let Some(h) = host {
        req = req.header("host", h);
    }
    if let Some(p) = proto {
        req = req.header("x-forwarded-proto", p);
    }
    let resp = futures_executor::block_on(app().oneshot(req.body(Body::empty()).unwrap())).unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = futures_executor::block_on(axum::body::to_bytes(resp.into_body(), usize::MAX)).unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let get = |k: &str| body["config"][k].as_str().unwrap_or_else(|| panic!("{k} in {body}")).to_string();
    [get("admin_base_url"), get("space_base_url"), get("app_base_url")]
}

fn host() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,10}(\\.[a-z][a-z0-9-]{0,10}){0,2}(:[0-9]{2,5})?"
}

proptest! {
    #![proptest_config(proptest_config(128))]

    #[test]
    fn prop_all_three_base_urls_are_the_origin_the_browser_used(
        h in host(),
        proto in prop::option::of(prop::sample::select(vec!["http", "https"])),
    ) {
        let want = format!("{}://{h}", proto.unwrap_or("http"));
        prop_assert_eq!(base_urls(Some(&h), proto), [want.clone(), want.clone(), want]);
    }

    /// Control for the property above: an untrusted scheme is never put into a URL, whatever it looks like.
    #[test]
    fn prop_an_untrusted_scheme_is_never_used(h in host(), proto in "[a-zA-Z][a-zA-Z0-9+.:-]{0,12}".prop_filter("not a trusted one", |p| p != "http" && p != "https")) {
        let want = format!("http://{h}");
        prop_assert_eq!(base_urls(Some(&h), Some(&proto)), [want.clone(), want.clone(), want]);
    }
}

/// Rule 8 for the properties above: the generators must reach a host with a port, a dotted host, and each scheme.
#[test]
fn prop_instance_strategies_reach_every_input_class() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let (mut port, mut dotted, mut plain) = (false, false, false);
    for _ in 0..128 {
        let h = host().new_tree(&mut runner).unwrap().current();
        port |= h.contains(':');
        dotted |= h.contains('.');
        plain |= !h.contains('.') && !h.contains(':');
    }
    assert!(port && dotted && plain, "[port, dotted, plain]: {:?}", [port, dotted, plain]);
}

/// The forwarded scheme as proxies send it: lower case, and a comma-separated chain with the client's side first.
#[rstest]
#[case::chain_https_first(Some("thrust.example.test"), Some("https, http"), "https://thrust.example.test")]
#[case::chain_http_first(Some("thrust.example.test"), Some("http, https"), "http://thrust.example.test")]
#[case::padded(Some("thrust.example.test"), Some("  https  "), "https://thrust.example.test")]
#[case::upper_case_is_not_trusted(Some("thrust.example.test"), Some("HTTPS"), "http://thrust.example.test")]
#[case::empty(Some("thrust.example.test"), Some(""), "http://thrust.example.test")]
#[case::no_host(None, Some("https"), "https://localhost")]
#[case::no_host_no_proto(None, None, "http://localhost")]
fn the_forwarded_scheme_and_the_host_make_the_origin(#[case] host: Option<&str>, #[case] proto: Option<&str>, #[case] want: &str) {
    assert_eq!(base_urls(host, proto), [want, want, want].map(String::from));
}
