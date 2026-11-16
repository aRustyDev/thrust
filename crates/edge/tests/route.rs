//! The routing rule and the request scrubbing (src/route.rs, with src/pin.rs and src/forward.rs on the path it takes).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions, not the helpers beside them"
)]
mod common;

use common::{block_on, proptest_config, FakeFlags, OTHER_PIN_KEY, PIN_KEY};
use proptest::prelude::*;
use rstest::rstest;
use serde_json::json;
use thrust_edge::forward;
use thrust_edge::pin::{self, Kind, Pin, COOKIE_NAME};
use thrust_edge::route::{self, alias_key, decide, target_key, Decision, FlagError};

const SUFFIX: &str = "-thrust.team.workers.dev";
const URL: &str = "https://v0-3-0-thrust.team.workers.dev";
const NOW: u64 = 1_800_000_000;

#[derive(Clone, Copy, Debug)]
enum Cookie {
    Absent,
    Valid,
    Forged,
    Expired,
}

#[derive(Clone, Copy, Debug)]
enum TargetFlag {
    Allowed,
    Denied,
    Missing,
}

#[derive(Clone, Copy, Debug)]
enum ClientHeaders {
    Clean,
    Spoofed,
}

fn pin_for(name: &str, exp: u64) -> Pin {
    Pin { principal: "ci.access".into(), kind: Kind::Ci, audience: "rc".into(), requested: name.into(), exp }
}

fn flags(routing: bool, target: TargetFlag) -> FakeFlags {
    let value =
        json!({ "url": URL, "deployment": "45171b81-2f57-4c5d", "release": "v0.3.0", "kind": "rc", "data": "shared" });
    FakeFlags {
        routing: Ok(routing),
        aliases: vec![],
        targets: match target {
            TargetFlag::Allowed => vec![(target_key("v0.3.0"), vec!["ci", "human"], Ok(value))],
            // the flag exists but its rules do not serve this kind: the default (empty) variation comes back
            TargetFlag::Denied => vec![(target_key("v0.3.0"), vec!["human"], Ok(value))],
            TargetFlag::Missing => vec![],
        },
    }
}

/// What a client sends: the request's own headers, and on the spoofed side everything a client could use to get past the rules.
fn client_headers(spoofed: ClientHeaders, cookie_value: Option<&str>) -> Vec<(String, String)> {
    let mut cookies = vec!["session=abc".to_string()];
    if let Some(v) = cookie_value {
        cookies.push(format!("{COOKIE_NAME}={v}"));
    }
    let mut h = vec![("Host".to_string(), "thrust.sow.foo".to_string()), ("Accept".to_string(), "*/*".to_string())];
    if let ClientHeaders::Spoofed = spoofed {
        cookies.push("CF_Authorization=forged.jwt.value".into());
        h.push(("Cf-Access-Jwt-Assertion".into(), "forged.jwt.value".into()));
        h.push(("CF-Access-Client-Id".into(), "x.access".into()));
        h.push(("cf-access-client-secret".into(), "y".into()));
        h.push(("Cloudflare-Workers-Version-Overrides".into(), "thrust=\"0123\"".into()));
        h.push(("Cloudflare-Workers-Version-Key".into(), "k".into()));
        h.push(("X-Forwarded-Host".into(), "evil.example".into()));
    }
    h.push(("Cookie".to_string(), cookies.join("; ")));
    h
}

/// The header names a client must never get to the upstream (lowercase), and the cookies.
fn forbidden(name: &str, value: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("cf-access-")
        || n.starts_with("cloudflare-workers-version-")
        || (n == "x-forwarded-host" && value != "thrust.sow.foo")
        || (n == "cookie" && (value.to_ascii_lowercase().contains("cf_authorization") || value.contains(COOKIE_NAME)))
}

/// The plan's matrix (ADR-0111): only a valid pin, with routing on and a target that allows its kind, reaches the target; every
/// other combination is production; and in every combination, nothing a client spoofed reaches the upstream.
#[rstest]
fn the_routing_matrix(
    #[values(Cookie::Absent, Cookie::Valid, Cookie::Forged, Cookie::Expired)] cookie: Cookie,
    #[values(true, false)] routing: bool,
    #[values(TargetFlag::Allowed, TargetFlag::Denied, TargetFlag::Missing)] target: TargetFlag,
    #[values(ClientHeaders::Clean, ClientHeaders::Spoofed)] client: ClientHeaders,
) {
    let value = match cookie {
        Cookie::Absent => None,
        Cookie::Valid => Some(pin::seal(&pin_for("v0.3.0", NOW + 60), PIN_KEY).unwrap()),
        Cookie::Forged => Some(pin::seal(&pin_for("v0.3.0", NOW + 60), OTHER_PIN_KEY).unwrap()),
        Cookie::Expired => Some(pin::seal(&pin_for("v0.3.0", NOW), PIN_KEY).unwrap()),
    };
    let headers = client_headers(client, value.as_deref());
    // the edge's own path: the Cookie header to a pin, then the rule
    let cookie_header = headers.iter().find(|(n, _)| n == "Cookie").map(|(_, v)| v.as_str()).unwrap();
    let pin = pin::from_cookie_header(cookie_header).and_then(|v| pin::open(v, PIN_KEY, NOW).ok());
    let got = block_on(decide(pin.as_ref(), &flags(routing, target), SUFFIX));

    let reaches = matches!(cookie, Cookie::Valid) && routing && matches!(target, TargetFlag::Allowed);
    match (&got, reaches) {
        (Decision::Target(t), true) => assert_eq!((t.name.as_str(), t.url.as_str()), ("v0.3.0", URL)),
        (Decision::Production, false) => {}
        _ => panic!("{cookie:?} routing={routing} {target:?}: got {got:?}"),
    }

    // whichever way it goes, the headers that leave are scrubbed; a target also gets the edge's forwarding headers
    let scrubbed = route::scrub(&headers);
    let leaving = match &got {
        Decision::Target(_) => forward::upstream_headers(&scrubbed, "thrust.sow.foo"),
        Decision::Production => scrubbed,
    };
    for (n, v) in &leaving {
        assert!(!forbidden(n, v), "{cookie:?} {client:?}: {n}: {v} reached the upstream");
    }
    assert!(leaving.iter().any(|(n, v)| n == "Cookie" && v == "session=abc"), "the client's own cookie still goes");
}

#[derive(Clone, Copy, Debug)]
enum Alias {
    None,
    ToTarget,
    ToProd,
    Invalid,
    /// an alias outside the name grammar whose key has a live target flag: the alias itself must be refused
    InvalidWithAFlag,
    Error,
}

/// Aliases: no alias flag uses the requested name; an alias naming a target routes there; an alias naming `prod`, an invalid
/// one or an error is production. Crossed with whether the requested name itself is a live target.
#[rstest]
fn aliases(
    #[values(Alias::None, Alias::ToTarget, Alias::ToProd, Alias::Invalid, Alias::InvalidWithAFlag, Alias::Error)] alias: Alias,
    #[values(true, false)] requested_is_a_target: bool,
) {
    let value = json!({ "url": URL, "deployment": "d1" });
    let mut f = FakeFlags {
        routing: Ok(true),
        aliases: vec![],
        targets: vec![(target_key("v0.3.0"), vec!["ci"], Ok(value.clone()))],
    };
    if requested_is_a_target {
        f.targets.push((target_key("staging"), vec!["ci"], Ok(json!({ "url": URL, "deployment": "d0" }))));
    }
    let answer = match alias {
        Alias::None => None,
        Alias::ToTarget => Some(Ok("v0.3.0".to_string())),
        Alias::ToProd => Some(Ok("prod".to_string())),
        Alias::Invalid => Some(Ok("Not A Name".to_string())),
        Alias::InvalidWithAFlag => {
            f.targets.push((target_key("V1"), vec!["ci"], Ok(json!({ "url": URL, "deployment": "d9" }))));
            Some(Ok("V1".to_string()))
        }
        Alias::Error => Some(Err(FlagError)),
    };
    if let Some(a) = answer {
        f.aliases.push((alias_key("staging"), a));
    }
    let got = block_on(decide(Some(&pin_for("staging", NOW + 60)), &f, SUFFIX));
    let want = match alias {
        Alias::None if requested_is_a_target => Some("d0"),
        Alias::ToTarget => Some("d1"),
        _ => None,
    };
    match (got, want) {
        (Decision::Target(t), Some(d)) => assert_eq!(t.deployment, d),
        (Decision::Production, None) => {}
        (got, want) => panic!("{alias:?} requested_is_a_target={requested_is_a_target}: got {got:?}, wanted {want:?}"),
    }
}

/// An error from Flagship at any step is production, never a target.
#[rstest]
fn a_flagship_error_is_production(#[values(0, 1, 2)] step: usize) {
    let value = json!({ "url": URL, "deployment": "d1" });
    let mut f =
        FakeFlags { routing: Ok(true), aliases: vec![], targets: vec![(target_key("v0.3.0"), vec!["ci"], Ok(value))] };
    match step {
        0 => f.routing = Err(FlagError),
        1 => f.aliases.push((alias_key("v0.3.0"), Err(FlagError))),
        _ => f.targets = vec![(target_key("v0.3.0"), vec!["ci"], Err(FlagError))],
    }
    assert_eq!(block_on(decide(Some(&pin_for("v0.3.0", NOW + 60)), &f, SUFFIX)), Decision::Production);
}

#[test]
fn no_pin_is_production_without_asking_flagship() {
    // a fake whose every answer is an error would make any question visible as production too, so use one that would route
    let f = FakeFlags {
        routing: Ok(true),
        aliases: vec![],
        targets: vec![(target_key("x"), vec!["ci"], Ok(json!({ "url": URL, "deployment": "d" })))],
    };
    assert_eq!(block_on(decide(None, &f, SUFFIX)), Decision::Production);
}

/// The target's kind and audience are what the target flag is evaluated with (its rules name kinds and audiences).
#[test]
fn the_flag_is_asked_with_the_pins_kind_and_audience() {
    let value = json!({ "url": URL, "deployment": "d1" });
    let f = FakeFlags {
        routing: Ok(true),
        aliases: vec![],
        targets: vec![(target_key("v0.3.0"), vec!["human"], Ok(value))],
    };
    let mut p = pin_for("v0.3.0", NOW + 60);
    assert_eq!(
        block_on(decide(Some(&p), &f, SUFFIX)),
        Decision::Production,
        "a ci pin is not served a human-only target"
    );
    p.kind = Kind::Human;
    assert!(matches!(block_on(decide(Some(&p), &f, SUFFIX)), Decision::Target(_)));
    assert_eq!(route::EvalContext::of(&p), route::EvalContext { kind: "human", audience: "rc".into() });
}

/// A target flag can send traffic only to a Preview of the Worker: `https://<label><suffix>` and nothing else.
#[rstest]
#[case::spec_preview("https://v0-3-0-thrust.team.workers.dev", true)]
#[case::dev_preview("https://dev-login-page-thrust.team.workers.dev", true)]
#[case::http("http://v0-3-0-thrust.team.workers.dev", false)]
#[case::path("https://v0-3-0-thrust.team.workers.dev/x", false)]
#[case::trailing_slash("https://v0-3-0-thrust.team.workers.dev/", false)]
#[case::port("https://v0-3-0-thrust.team.workers.dev:8443", false)]
#[case::userinfo("https://u@v0-3-0-thrust.team.workers.dev", false)]
#[case::other_host("https://evil.example", false)]
#[case::suffix_as_prefix("https://x-thrust.team.workers.dev.evil.example", false)]
#[case::empty_label("https://-thrust.team.workers.dev", false)]
#[case::label_with_dot("https://a.b-thrust.team.workers.dev", false)]
#[case::uppercase("https://V0-thrust.team.workers.dev", false)]
fn target_urls(#[case] url: &str, #[case] ok: bool) {
    let got = route::parse_target("v0.3.0", &json!({ "url": url, "deployment": "d1" }), SUFFIX);
    assert_eq!(got.is_some(), ok, "{url}");
    if let Some(target) = got {
        assert_eq!(target.url, url);
        assert_eq!(target.host(), url.strip_prefix("https://").unwrap(), "the host the edge fetches and rewrites from");
    }
}

/// A misconfigured suffix makes nothing a target: it must be non-empty, start with `-` (the Preview name comes first) and be
/// lowercase.
#[rstest]
#[case::empty("", "https://x")]
#[case::no_leading_dash("thrust.team.workers.dev", "https://xthrust.team.workers.dev")]
#[case::uppercase("-Thrust.team.workers.dev", "https://x-Thrust.team.workers.dev")]
fn a_misconfigured_suffix_makes_nothing_a_target(#[case] suffix: &str, #[case] url: &str) {
    assert_eq!(route::parse_target("x", &json!({ "url": url, "deployment": "d1" }), suffix), None);
}

/// ADR-0111 names the keys: `alias-staging`, and `target-<name>` with Flagship's alphabet (no `.`).
#[test]
fn spec_flag_keys() {
    assert_eq!(alias_key("staging"), "alias-staging");
    assert_eq!(target_key("v0.3.0"), "target-v0_3_0");
    assert_eq!(target_key("dev-login-page"), "target-dev-login-page");
}

#[rstest]
#[case::no_url(json!({ "deployment": "d1" }))]
#[case::no_deployment(json!({ "url": URL }))]
#[case::empty_deployment(json!({ "url": URL, "deployment": "" }))]
#[case::odd_deployment(json!({ "url": URL, "deployment": "d 1" }))]
#[case::url_not_a_string(json!({ "url": 1, "deployment": "d1" }))]
#[case::empty_object(json!({}))]
fn incomplete_target_values_are_no_target(#[case] value: serde_json::Value) {
    assert_eq!(route::parse_target("v0.3.0", &value, SUFFIX), None);
}

proptest! {
    #![proptest_config(proptest_config(512))]

    /// Flag keys: within Flagship's 64 characters and alphabet, and distinct names never share a key.
    #[test]
    fn prop_flag_keys_fit_flagship_and_are_one_to_one(a in "[a-z0-9][a-z0-9.-]{0,56}", b in "[a-z0-9][a-z0-9.-]{0,56}") {
        for k in [alias_key(&a), target_key(&a)] {
            prop_assert!(k.len() <= 64, "{}", k);
            prop_assert!(k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'), "{}", k);
        }
        prop_assert_eq!(target_key(&a) == target_key(&b), a == b);
        prop_assert_ne!(alias_key(&a), target_key(&b));
        // the reference: the prefix, a dash, the name with every '.' as '_'
        let mapped: String = a.chars().map(|c| if c == '.' { '_' } else { c }).collect();
        prop_assert_eq!(alias_key(&a), format!("alias-{mapped}"));
        prop_assert_eq!(target_key(&a), format!("target-{mapped}"));
    }

    /// A target URL whose host is not `<label><suffix>` is never a target, whatever it is.
    #[test]
    fn prop_only_preview_hosts_are_targets(host in "[a-z0-9.:@/-]{1,60}") {
        let url = format!("https://{host}");
        let got = route::parse_target("x", &json!({ "url": url, "deployment": "d" }), SUFFIX);
        let label_ok = host.strip_suffix(SUFFIX).is_some_and(|l| {
            !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !l.starts_with('-') && !l.ends_with('-')
        });
        prop_assert_eq!(got.is_some(), label_ok, "{}", url);
    }

    /// Scrubbing drops every forbidden header and cookie, whatever their case, and keeps everything else in order.
    #[test]
    fn prop_scrubbing(
        headers in prop::collection::vec((
            prop_oneof![
                "[A-Za-z][A-Za-z0-9-]{0,15}",
                Just("CF-Access-Client-Id".to_string()), Just("cf-access-jwt-assertion".to_string()),
                Just("Cloudflare-Workers-Version-Overrides".to_string()), Just("X-Forwarded-Host".to_string()),
                Just("x-forwarded-proto".to_string()),
            ],
            "[ -~]{0,20}",
        ), 0..8),
        cookies in prop::collection::vec(prop_oneof![
            ("[A-Za-z][A-Za-z0-9_]{0,10}", "[A-Za-z0-9]{0,8}"),
            (Just("CF_Authorization".to_string()), "[A-Za-z0-9]{1,8}"),
            (Just("cf_authorization".to_string()), "[A-Za-z0-9]{1,8}"),
            (Just(COOKIE_NAME.to_string()), "[A-Za-z0-9]{1,8}"),
        ], 0..6),
    ) {
        let mut all = headers.clone();
        let cookie_value = cookies.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; ");
        all.push(("Cookie".to_string(), cookie_value));
        let out = route::scrub(&all);
        let kept_cookies: Vec<String> = cookies.iter()
            .filter(|(n, _)| !n.eq_ignore_ascii_case("CF_Authorization") && n != COOKIE_NAME)
            .map(|(n, v)| format!("{n}={v}"))
            .collect();
        let want: Vec<(String, String)> = headers.iter().filter(|(n, _)| {
            let l = n.to_ascii_lowercase();
            !(l.starts_with("cf-access-") || l.starts_with("cloudflare-workers-version-") || l == "x-forwarded-host" || l == "x-forwarded-proto"
                || l == "cookie")
        }).cloned().chain((!kept_cookies.is_empty()).then(|| ("Cookie".to_string(), kept_cookies.join("; ")))).collect();
        let got: Vec<(String, String)> = out.into_iter().filter(|(n, _)| !n.eq_ignore_ascii_case("cookie") || n == "Cookie").collect();
        let want: Vec<(String, String)> = want.into_iter().filter(|(n, _)| !n.eq_ignore_ascii_case("cookie") || n == "Cookie").collect();
        prop_assert_eq!(got, want);
    }
}
