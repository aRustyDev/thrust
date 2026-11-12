//! What must hold of `plane_500` for every cause (#51): the answer is the kind's fixed 500 and the cause is only ever in the
//! response extension, where the composition root logs it from. The cause is free text (a database error, an upstream message,
//! a path), so it is generated, not listed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

use axum::body::Body;
use axum::http::Response;
use proptest::prelude::*;
use thrust_api::InternalErrorDetail;
use thrust_plane_types::{plane_500, Plane500};

/// `PROPTEST_CASES` sets the case count; failures are persisted next to the tests, as everywhere else in the workspace.
fn proptest_config(default_cases: u32) -> proptest::test_runner::Config {
    use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence};
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_cases);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config { cases, failure_persistence, ..Config::default() }
}

fn kind() -> impl Strategy<Value = Plane500> {
    prop_oneof![
        Just(Plane500::SomethingWentWrong),
        Just(Plane500::UnexpectedError),
        Just(Plane500::InternalServerError)
    ]
}

fn body_of(resp: Response<Body>) -> (Response<Body>, Vec<u8>) {
    let (parts, body) = resp.into_parts();
    let bytes = futures_executor::block_on(axum::body::to_bytes(body, usize::MAX)).unwrap();
    (Response::from_parts(parts, Body::empty()), bytes.to_vec())
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// Whatever the cause says (quotes, braces, a copy of the pin's own text, a multi-byte character), status, headers and body
    /// are those of the same kind given an empty cause, and the body is exactly `{"error":"<the kind's text>"}`.
    #[test]
    fn prop_plane_500_never_leaks_the_cause(kind in kind(), cause in any::<String>()) {
        let (resp, body) = body_of(plane_500(kind, cause.clone()));
        let (reference, reference_body) = body_of(plane_500(kind, ""));

        prop_assert_eq!(resp.status(), reference.status());
        prop_assert_eq!(resp.headers(), reference.headers());
        prop_assert_eq!(&body, &reference_body);
        prop_assert_eq!(body, format!(r#"{{"error":"{}"}}"#, kind.message()).into_bytes());

        let detail = resp.extensions().get::<InternalErrorDetail>().expect("the cause is attached for the log");
        prop_assert_eq!(&detail.0, &cause);
    }
}
