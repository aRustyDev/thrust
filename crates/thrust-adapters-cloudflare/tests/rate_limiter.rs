//! The parts of the Cloudflare rate limiter that are plain functions (issue #43): which binding a limiter key belongs to, what the
//! binding's answer means to the port, and which windows the binding can count over.
//!
//! The binding itself is a JS object, so `CfRateLimiter::hit` cannot run off wasm32; it runs in the `worker-cf` cells of the
//! contract suite (`create_is_rate_limited_per_user` and every sign-in), which would fail if a key reached no binding. What is
//! here is what decides the answer once the binding has spoken.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use common::proptest_config;
use proptest::prelude::*;
use rstest::rstest;
use thrust_adapters_cloudflare::{check_binding_period, decision, KeyClass, BINDING_PERIODS_SECS};
use thrust_ports::RateDecision;

/// The three classes and the text the core and the v1 routes put before the `:` of their keys (`create:<user>` in
/// `thrust-core/src/lib.rs`, `signin:<email>` in `thrust-core/src/auth.rs`, `v1:<api key>` in `thrust-api-v1/src/ctx.rs`).
const CLASSES: [(&str, KeyClass); 3] =
    [("create", KeyClass::Create), ("signin", KeyClass::SignIn), ("v1", KeyClass::V1)];

// ---------------------------------------------------------------------------------------------
// KeyClass::of
// ---------------------------------------------------------------------------------------------

/// Every class, with every shape of what follows its colon (a uuid, an email, text with colons of its own, nothing): the class
/// is read from the text before the FIRST colon and nothing after it matters.
#[rstest]
fn a_key_of_a_known_class_is_routed_to_its_binding(
    #[values(0, 1, 2)] class: usize,
    #[values("", "u", "0192f3a4-0000-7000-8000-000000000001", "a@example.test", "a:b", "::", " ", "create:", "v1:x")]
    rest: &str,
) {
    let (prefix, want) = CLASSES[class];
    assert_eq!(KeyClass::of(&format!("{prefix}:{rest}")), Some(want), "{prefix}:{rest}");
}

/// A key of no class gets no binding, and in particular is not matched by a prefix, a different case, a longer word, or a class
/// name with no colon after it.
#[rstest]
fn a_key_of_no_class_is_routed_nowhere(
    #[values(
        "",
        "create",
        "signin",
        "v1",
        "Create:x",
        "CREATE:x",
        "creates:x",
        "create :x",
        " create:x",
        "createx:y",
        "v1x:y",
        "v2:y",
        "V1:y",
        "sign-in:x",
        "signin2:x",
        "api:x",
        ":x",
        ":",
        "x:create:y",
        "createsignin:x",
        "plain"
    )]
    key: &str,
) {
    assert_eq!(KeyClass::of(key), None, "{key:?}");
}

proptest! {
    #![proptest_config(proptest_config(512))]

    /// Over any text at all: a key has a class exactly when it starts with one of the three class names and a colon.
    #[test]
    fn prop_a_key_has_a_class_exactly_when_it_starts_with_a_class_and_a_colon(key in any::<String>()) {
        let starts = CLASSES.iter().any(|(p, _)| key.strip_prefix(p).is_some_and(|r| r.starts_with(':')));
        prop_assert_eq!(KeyClass::of(&key).is_some(), starts, "{:?}", key);
    }

    /// Over any text after the colon, a class's key keeps its class.
    #[test]
    fn prop_what_follows_the_colon_never_changes_the_class(class in 0..3usize, rest in any::<String>()) {
        let (prefix, want) = CLASSES[class];
        prop_assert_eq!(KeyClass::of(&format!("{prefix}:{rest}")), Some(want));
    }
}

// ---------------------------------------------------------------------------------------------
// decision
// ---------------------------------------------------------------------------------------------

/// The whole of the small domain: success or refusal, over limits that include none, one, and the largest, and windows that
/// include none, the two the binding has, and the larger ones a self-hosted setting could name.
#[rstest]
fn the_bindings_answer_as_the_port_reads_it(
    #[values(true, false)] success: bool,
    #[values(0, 1, 2, 30, 1000, u32::MAX)] limit: u32,
    #[values(0, 1, 10, 60, 3600, u64::MAX)] window: u64,
) {
    let got = decision(success, limit, window);
    if success {
        // `limit - 1`, and never below zero: the upper bound of what could be left (see `decision`)
        assert_eq!(got, RateDecision::Allowed { remaining: limit.saturating_sub(1) });
    } else {
        // one whole period, and never zero: a `Retry-After: 0` would invite an immediate retry into the same refusal
        assert_eq!(got, RateDecision::Limited { retry_after_secs: window.max(1) });
    }
}

proptest! {
    #![proptest_config(proptest_config(512))]

    /// Over any limit and window: an allowance never claims more is left than the limit, and never that the whole of it is, so a
    /// client is not told its allowance is spent by an answer that let it through; a refusal always says to wait. Half of the
    /// limits and windows are drawn from 0 to 3, where the edges (none, one) are; a random number never lands there.
    #[test]
    fn prop_an_allowance_is_an_upper_bound_and_a_refusal_always_waits(
        limit in prop_oneof![0..4u32, any::<u32>()],
        window in prop_oneof![0..4u64, any::<u64>()],
    ) {
        match decision(true, limit, window) {
            RateDecision::Allowed { remaining } => {
                prop_assert!(remaining <= limit);
                prop_assert!(limit == 0 || remaining < limit);
            }
            RateDecision::Limited { .. } => prop_assert!(false, "a success is allowed"),
        }
        match decision(false, limit, window) {
            RateDecision::Limited { retry_after_secs } => prop_assert!(retry_after_secs >= 1),
            RateDecision::Allowed { .. } => prop_assert!(false, "a refusal is limited"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// check_binding_period
// ---------------------------------------------------------------------------------------------

#[rstest]
fn the_two_periods_the_binding_has_are_accepted(#[values(10, 60)] secs: u64) {
    assert_eq!(check_binding_period("THRUST_RATE_WINDOW_SECS", secs), Ok(()));
}

/// The neighbours of both periods, the period of the old Durable Object limiter's test setting, and Plane's other spellings
/// (`second`, `hour`, `day`).
#[rstest]
fn any_other_period_is_refused_by_the_setting_that_named_it(
    #[values(0, 1, 9, 11, 59, 61, 600, 3600, 86_400, u64::MAX)] secs: u64,
) {
    let err = check_binding_period("THRUST_RATE_WINDOW_SECS", secs).unwrap_err();
    assert!(err.contains("THRUST_RATE_WINDOW_SECS"), "{err}");
    assert!(err.contains(&format!("{secs} s")), "{err}");
}

proptest! {
    #![proptest_config(proptest_config(512))]

    /// Over any window: it is accepted exactly when it is one of the binding's two periods, and the list the adapter publishes is
    /// that rule. A random `u64` almost never lands on 10 or 60, so half of the windows are drawn from the small numbers around
    /// them: without that, this could not tell a wrong period in the list from the right one.
    #[test]
    fn prop_a_period_is_accepted_exactly_when_it_is_one_the_binding_has(secs in prop_oneof![0..200u64, any::<u64>()]) {
        prop_assert_eq!(check_binding_period("X", secs).is_ok(), secs == 10 || secs == 60);
        prop_assert_eq!(check_binding_period("X", secs).is_ok(), BINDING_PERIODS_SECS.contains(&secs));
    }
}
