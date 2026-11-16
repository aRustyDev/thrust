//! The internal API's 500 (#51): an internal failure is the pin's catch-all, `{"error":"Something went wrong please try again later"}`,
//! with the cause on the response for the log and never in the body. Before this a failure fell to `thrust-api`'s own shape,
//! `{"error":{"code":"internal",...}}`, an object where the React client reads a string (research/0007 PQ-107).
//!
//! What must hold: `Internal` and `Upstream` are the Plane 500 however the error was built (`?` on a `CoreError`, or a
//! `PlaneError::Other` made by hand); every other `CoreError` answers exactly what it answered before; and the cause, whatever
//! it says, is not in the body.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

use axum::body::Body;
use axum::http::{HeaderMap, Response};
use axum::response::IntoResponse;
use proptest::prelude::*;
use thrust_api::{ApiError, InternalErrorDetail};
use thrust_compat_plane::PlaneError;
use thrust_core::CoreError;
use thrust_plane_types::Plane500;

/// What a response is: status, headers, body bytes and the cause on it, if any.
struct Answer {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    cause: Option<String>,
}

fn answer(resp: Response<Body>) -> Answer {
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let cause = resp.extensions().get::<InternalErrorDetail>().map(|d| d.0.clone());
    let body = futures_executor::block_on(axum::body::to_bytes(resp.into_body(), usize::MAX)).unwrap().to_vec();
    Answer { status, headers, body, cause }
}

fn plane_body(kind: Plane500) -> Vec<u8> {
    format!(r#"{{"error":"{}"}}"#, kind.message()).into_bytes()
}

/// Every `CoreError`, with the cause it must carry if it is one the pin answers with its catch-all 500. Built fresh on each
/// call (a `CoreError` is not assumed to be `Clone`), so a test can feed one copy in and keep another as the reference.
fn variants() -> Vec<(CoreError, Option<&'static str>)> {
    let all = vec![
        (CoreError::Unauthenticated, None),
        (CoreError::Forbidden, None),
        (CoreError::Csrf, None),
        (CoreError::InvalidApiKey, None),
        (CoreError::NotFound, None),
        (CoreError::Conflict("taken".into()), None),
        (CoreError::RateLimited { retry_after_secs: 7 }, None),
        (CoreError::Invalid("bad".into()), None),
        (CoreError::TooLarge, None),
        (CoreError::Upstream("idp down".into()), Some("idp down")),
        (CoreError::Internal("db gone".into()), Some("db gone")),
    ];
    // A new variant is a compile error here until it is listed above and decided: is it the pin's catch-all, or not?
    for (e, _) in &all {
        match e {
            CoreError::Unauthenticated
            | CoreError::Forbidden
            | CoreError::Csrf
            | CoreError::InvalidApiKey
            | CoreError::NotFound
            | CoreError::Conflict(_)
            | CoreError::RateLimited { .. }
            | CoreError::Invalid(_)
            | CoreError::TooLarge
            | CoreError::Upstream(_)
            | CoreError::Internal(_) => {}
        }
    }
    all
}

#[test]
fn an_internal_failure_is_the_planes_500_however_the_error_was_built() {
    for (e, cause) in variants().into_iter().filter(|(_, c)| c.is_some()) {
        let cause = cause.unwrap();
        let label = e.to_string();
        // built by hand, as the assets stage does
        let by_hand = answer(PlaneError::Other(e).into_response());
        assert_eq!((by_hand.status, &by_hand.body), (500, &plane_body(Plane500::SomethingWentWrong)), "{label}");
        assert_eq!(by_hand.cause.as_deref(), Some(cause), "{label}: the cause is for the log");
    }
    // through `?` on a CoreError, as every handler does
    for (e, cause) in variants().into_iter().filter(|(_, c)| c.is_some()) {
        let cause = cause.unwrap();
        let label = e.to_string();
        let through_from = answer(PlaneError::from(e).into_response());
        assert_eq!(
            (through_from.status, &through_from.body),
            (500, &plane_body(Plane500::SomethingWentWrong)),
            "{label}"
        );
        assert_eq!(through_from.cause.as_deref(), Some(cause), "{label}");
    }
}

/// Every other `CoreError` answers what `thrust-api` answers, byte for byte and header for header: this change moved the 500
/// and nothing else.
#[test]
fn every_other_failure_answers_as_it_did() {
    let mut checked = 0;
    for ((e, cause), (reference, _)) in variants().into_iter().zip(variants()) {
        if cause.is_some() {
            continue;
        }
        let label = e.to_string();
        let got = answer(PlaneError::Other(e).into_response());
        let want = answer(ApiError(reference).into_response());
        assert_eq!((got.status, &got.body, &got.headers), (want.status, &want.body, &want.headers), "{label}");
        assert_eq!(got.cause, None, "{label}");
        checked += 1;
    }
    // not vacuous: nine of the eleven variants are not the catch-all
    assert_eq!(checked, 9);
}

/// Control (AGENTS.md testing rule 8): the comparison can tell the old shape from the new. `thrust-api`'s own 500 is not the
/// pin's, so a mapping that left `Internal` to fall through would fail the first test, not pass it.
#[test]
fn control_thrust_apis_own_500_is_not_the_planes() {
    let thrust_shaped = answer(ApiError(CoreError::Internal("db gone".into())).into_response());
    assert_eq!(thrust_shaped.status, 500);
    assert_ne!(thrust_shaped.body, plane_body(Plane500::SomethingWentWrong));
    // and an identity provider that is down was a 502 there
    let upstream = answer(ApiError(CoreError::Upstream("idp down".into())).into_response());
    assert_eq!(upstream.status, 502);
}

#[test]
fn a_deliberate_500_names_its_rule_and_is_the_catch_all() {
    let got = answer(PlaneError::internal("per_page <= 0").into_response());
    assert_eq!((got.status, &got.body), (500, &plane_body(Plane500::SomethingWentWrong)));
    assert_eq!(got.cause.as_deref(), Some("per_page <= 0"));
    let other_kind =
        answer(PlaneError::Plane500 { kind: Plane500::UnexpectedError, cause: "x".into() }.into_response());
    assert_eq!(other_kind.body, plane_body(Plane500::UnexpectedError));
}

proptest! {
    /// Whatever an internal failure says (a database error, a path, a copy of the pin's own text), the body is the same fixed bytes.
    #[test]
    fn prop_an_internal_cause_never_reaches_the_body(cause in any::<String>(), upstream in any::<bool>()) {
        let e = if upstream { CoreError::Upstream(cause.clone()) } else { CoreError::Internal(cause.clone()) };
        let got = answer(PlaneError::from(e).into_response());
        prop_assert_eq!(got.status, 500);
        prop_assert_eq!(got.body, plane_body(Plane500::SomethingWentWrong));
        prop_assert_eq!(got.cause, Some(cause));
    }
}
