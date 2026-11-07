//! What every v1 route does before its own work (issue #58): read the API key, check the permission, count the request, and
//! report what is left. Pinned by the oracle's corpus (`errors.json`), driven through probe routes of this file so that none of it
//! depends on a real route.
//!
//! The order is DRF's: authentication, then permissions, then throttling, then the handler. So: a missing key is a 401 and an
//! unusable one a 403 before anything else; a caller who may not do the thing gets a 403 that is not counted and has no rate
//! headers; every answer after that, an error included, has them; and a 429 has `Retry-After` and not them.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use common::*;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Value};
use thrust_api_v1::{Ctx, PlaneUuid, V1Config, V1Error, V1Path};
use thrust_contracts::{ProjectId, Role, Uuid};
use thrust_core::{CoreError, Permission};

const DETAIL_NO_KEY: &str = "Authentication credentials were not provided.";
const DETAIL_BAD_KEY: &str = "Given API token is not valid";
const DETAIL_DENIED: &str = "You do not have permission to perform this action.";

// --- probe routes: one for each thing a real route does with `Ctx` -----------------------------------------------------------

async fn read(V1Path((slug, project)): V1Path<(String, PlaneUuid)>, ctx: Ctx) -> Result<Json<Value>, V1Error> {
    ctx.project_scope(&slug, ProjectId(project.0), Permission::ReadProject).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn write(V1Path((slug, project)): V1Path<(String, PlaneUuid)>, ctx: Ctx) -> Result<Json<Value>, V1Error> {
    ctx.project_scope(&slug, ProjectId(project.0), Permission::CreateWorkItem).await?;
    Ok(Json(json!({ "ok": true })))
}

/// Past the throttle, the handler itself fails: the answer must still carry the rate headers.
async fn gone(V1Path((slug, project)): V1Path<(String, PlaneUuid)>, ctx: Ctx) -> Result<Json<Value>, V1Error> {
    ctx.project_scope(&slug, ProjectId(project.0), Permission::ReadProject).await?;
    Err(V1Error::NotFound)
}

async fn workspace(V1Path(slug): V1Path<String>, ctx: Ctx) -> Result<Json<Value>, V1Error> {
    ctx.workspace_scope(&slug).await?;
    Ok(Json(json!({ "ok": true })))
}

fn probes() -> Router {
    Router::new()
        .route("/api/v1/workspaces/{slug}/projects/{project}/read/", get(read))
        .route("/api/v1/workspaces/{slug}/projects/{project}/write/", post(write))
        .route("/api/v1/workspaces/{slug}/projects/{project}/gone/", get(gone))
        .route("/api/v1/workspaces/{slug}/probe/", get(workspace))
}

fn uri(slug: &str, project: u128, tail: &str) -> String {
    format!("/api/v1/workspaces/{slug}/projects/{}/{tail}/", Uuid::from_u128(project).hyphenated())
}

fn read_uri() -> String {
    uri(SLUG, PROJECT, "read")
}

fn big_allowance() -> V1Config {
    V1Config { rate_limit: 10_000, rate_window_secs: 60 }
}

// --- authentication -----------------------------------------------------------------------------------------------------

/// The key states the corpus records (`errors.json`: no-api-key, bogus-api-key, expired-api-key, inactive-api-key), and the
/// two the pin treats alike (an empty header is no key; a revoked key is one nothing matches). Only a MISSING key is a 401.
#[rstest]
#[case::missing(None, 401, DETAIL_NO_KEY)]
#[case::empty(Some(""), 401, DETAIL_NO_KEY)]
#[case::unknown(Some("no-such-key"), 403, DETAIL_BAD_KEY)]
#[case::expired(Some("key-expired"), 403, DETAIL_BAD_KEY)]
#[case::revoked(Some("key-revoked"), 403, DETAIL_BAD_KEY)]
#[case::user_switched_off(Some("key-inactive"), 403, DETAIL_BAD_KEY)]
fn a_key_that_does_not_authenticate_is_refused_with_no_rate_headers(
    #[case] key: Option<&str>,
    #[case] status: u16,
    #[case] detail: &str,
) {
    let w = world(probes(), big_allowance());
    let headers: Vec<(&str, &str)> = key.map(|k| ("x-api-key", k)).into_iter().collect();
    let r = send(&w.router, "GET", &read_uri(), &headers);
    assert_eq!((r.status, &r.body), (status, &json!({ "detail": detail })));
    assert!(!r.has_rate_headers(), "{:?}", r.headers);
}

/// The control for the table above: the same request with a good key passes, so the refusals are the key's doing.
#[test]
fn a_good_key_passes_and_is_counted() {
    let w = world(probes(), big_allowance());
    let r = send_as(&w.router, "key-admin", "GET", &read_uri());
    assert_eq!((r.status, &r.body), (200, &json!({ "ok": true })));
    assert!(r.has_rate_headers());
}

/// A session is not a credential on v1: the bearer token and the cookie that sign a person in to the app mean nothing here.
#[test]
fn a_session_is_not_a_credential() {
    let w = world(probes(), big_allowance());
    let bearer = send(&w.router, "GET", &read_uri(), &[("authorization", "Bearer session-admin")]);
    assert_eq!((bearer.status, &bearer.body), (401, &json!({ "detail": DETAIL_NO_KEY })));
    let cookie = send(&w.router, "GET", &read_uri(), &[("cookie", "__Host-thrust_session=session-admin")]);
    assert_eq!((cookie.status, &cookie.body), (401, &json!({ "detail": DETAIL_NO_KEY })));
    // and a good key is not spoiled by one beside it
    let both = send(&w.router, "GET", &read_uri(), &[("x-api-key", "key-admin"), ("authorization", "Bearer nonsense")]);
    assert_eq!(both.status, 200);
}

// --- permissions --------------------------------------------------------------------------------------------------------

/// Every caller against every kind of route, a project that is there and one that is not, a workspace that is there and one
/// that is not: the pin's permission classes find no membership in any case but one, and say 403 for all of them.
#[test]
fn only_a_member_of_the_project_with_enough_role_gets_in() {
    let w = world(probes(), big_allowance());
    for who in Who::ALL {
        // The rule as the pin states it (`ProjectEntityPermission`): any member reads, a member or an admin writes. Written out here,
        // not asked of core, so the test is an independent statement of it.
        for (tail, min_role) in [("read", Role::Guest), ("write", Role::Member)] {
            for project in [PROJECT, PROJECT + 1] {
                for slug in [SLUG, "elsewhere"] {
                    let method = if tail == "write" { "POST" } else { "GET" };
                    let r = send_as(&w.router, who.key(), method, &uri(slug, project, tail));
                    let allowed = slug == SLUG && project == PROJECT && who.role().is_some_and(|role| role >= min_role);
                    let case = format!("{who:?} {method} {tail} project={project:#x} slug={slug}");
                    if allowed {
                        assert_eq!((r.status, &r.body), (200, &json!({ "ok": true })), "{case}");
                        assert!(r.has_rate_headers(), "{case}");
                    } else {
                        // the account that was switched off never gets as far as a permission
                        let detail = if who == Who::Inactive { DETAIL_BAD_KEY } else { DETAIL_DENIED };
                        assert_eq!((r.status, &r.body), (403, &json!({ "detail": detail })), "{case}");
                        assert!(!r.has_rate_headers(), "a refused request is not counted: {case}");
                    }
                }
            }
        }
    }
}

#[test]
fn a_workspace_route_is_for_its_members() {
    let w = world(probes(), big_allowance());
    for who in Who::ALL {
        for slug in [SLUG, "elsewhere"] {
            let r = send_as(&w.router, who.key(), "GET", &format!("/api/v1/workspaces/{slug}/probe/"));
            let allowed = slug == SLUG && who.role().is_some();
            let detail = if who == Who::Inactive { DETAIL_BAD_KEY } else { DETAIL_DENIED };
            if allowed {
                assert_eq!((r.status, r.has_rate_headers()), (200, true), "{who:?} {slug}");
            } else {
                assert_eq!((r.status, &r.body), (403, &json!({ "detail": detail })), "{who:?} {slug}");
                assert!(!r.has_rate_headers(), "{who:?} {slug}");
            }
        }
    }
}

// --- the allowance ------------------------------------------------------------------------------------------------------

/// `5/minute` as the corpus records it (`rate_limit.json`): the count goes 4, 3, 2, 1, 0 and the sixth is a 429.
#[test]
fn the_allowance_counts_down_and_the_next_request_is_a_429() {
    let w = world(probes(), V1Config { rate_limit: 5, rate_window_secs: 60 });
    let mut seen = Vec::new();
    for _ in 0..5 {
        let r = send_as(&w.router, "key-admin", "GET", &read_uri());
        assert_eq!(r.status, 200);
        // `X-RateLimit-Reset` is a Unix time a whole window ahead of the request, as the pin computes it
        assert_eq!(r.header("x-ratelimit-reset"), Some((NOW + 60).to_string().as_str()));
        seen.push(r.header("x-ratelimit-remaining").unwrap().to_owned());
    }
    assert_eq!(seen, ["4", "3", "2", "1", "0"]);

    let limited = send_as(&w.router, "key-admin", "GET", &read_uri());
    assert_eq!(
        (limited.status, &limited.body),
        (429, &json!({ "error_code": 5900, "error_message": "RATE_LIMIT_EXCEEDED" }))
    );
    let retry: u64 = limited.header("retry-after").expect("Retry-After").parse().unwrap();
    assert!((1..=60).contains(&retry), "{retry}");
    assert!(!limited.has_rate_headers(), "a 429 has Retry-After and not the rate headers");
}

/// The key is the allowance, not the person: the admin's second key has a count of its own, and so has everyone else.
#[test]
fn each_key_has_its_own_allowance() {
    let w = world(probes(), V1Config { rate_limit: 2, rate_window_secs: 60 });
    for _ in 0..2 {
        assert_eq!(send_as(&w.router, "key-admin", "GET", &read_uri()).status, 200);
    }
    assert_eq!(send_as(&w.router, "key-admin", "GET", &read_uri()).status, 429);
    let second = send_as(&w.router, "key-admin-2", "GET", &read_uri());
    assert_eq!((second.status, second.header("x-ratelimit-remaining")), (200, Some("1")));
    let other = send_as(&w.router, "key-member", "GET", &read_uri());
    assert_eq!((other.status, other.header("x-ratelimit-remaining")), (200, Some("1")));
}

/// A refused request is not counted: the throttle comes after the permission, as in DRF, so a caller who is turned away cannot
/// spend their allowance (and so a run of refusals is not a 429).
#[test]
fn a_refused_request_does_not_spend_the_allowance() {
    let w = world(probes(), V1Config { rate_limit: 2, rate_window_secs: 60 });
    for _ in 0..5 {
        let r = send_as(&w.router, "key-guest", "POST", &uri(SLUG, PROJECT, "write"));
        assert_eq!(r.status, 403);
    }
    let r = send_as(&w.router, "key-guest", "GET", &read_uri());
    assert_eq!((r.status, r.header("x-ratelimit-remaining")), (200, Some("1")));
}

proptest! {
    #![proptest_config(proptest_config(24))]

    /// For any allowance and any number of requests: the `i`th is let through with `limit - i` left while `i <= limit`, and every
    /// one after is a 429 with no rate headers.
    #[test]
    fn prop_the_count_is_the_limit_less_the_requests_so_far(limit in 1u32..12, calls in 1u32..30) {
        let w = world(probes(), V1Config { rate_limit: limit, rate_window_secs: 60 });
        for i in 1..=calls {
            let r = send_as(&w.router, "key-member", "GET", &read_uri());
            if i <= limit {
                prop_assert_eq!(r.status, 200);
                let left = (limit - i).to_string();
                prop_assert_eq!(r.header("x-ratelimit-remaining"), Some(left.as_str()));
            } else {
                prop_assert_eq!(r.status, 429);
                prop_assert!(!r.has_rate_headers());
            }
        }
    }
}

/// Past the throttle the handler can still fail, and its answer says what is left: the headers ride every answer that was counted.
#[test]
fn an_error_after_the_throttle_carries_the_rate_headers() {
    let w = world(probes(), V1Config { rate_limit: 3, rate_window_secs: 60 });
    let r = send_as(&w.router, "key-admin", "GET", &uri(SLUG, PROJECT, "gone"));
    assert_eq!((r.status, &r.body), (404, &json!({ "error": "The requested resource does not exist." })));
    assert_eq!(r.header("x-ratelimit-remaining"), Some("2"));
    assert_eq!(r.header("x-ratelimit-reset"), Some((NOW + 60).to_string().as_str()));
}

// --- paths that do not match --------------------------------------------------------------------------------------------

/// A path that no route matches is the URLconf's 404, answered before any credential is read: a missing key does not make it a 401.
/// An id that is not a uuid, in Django's sense, is such a path (the converter takes lower-case canonical form only).
#[rstest]
#[case::not_a_uuid("nope")]
#[case::upper_case("00000000-0000-0000-0000-0000000000A1")]
#[case::bare_hex("000000000000000000000000000000a1")]
#[case::braced("{00000000-0000-0000-0000-0000000000a1}")]
#[case::too_short("00000000-0000-0000-0000-0000000000a")]
fn an_id_that_is_not_a_uuid_is_the_urlconfs_404(#[case] id: &str) {
    let w = world(probes(), big_allowance());
    let path = format!("/api/v1/workspaces/{SLUG}/projects/{id}/read/");
    for headers in [vec![], vec![("x-api-key", "key-admin")]] {
        let r = send(&w.router, "GET", &path, &headers);
        assert_eq!((r.status, &r.body), (404, &json!({ "error": "Page not found." })), "{headers:?}");
        assert!(!r.has_rate_headers());
    }
}

#[test]
fn an_unknown_path_under_v1_is_the_urlconfs_404() {
    let w = world(probes(), big_allowance());
    for path in ["/api/v1/nope/", "/api/v1/workspaces/", "/api/v1/workspaces/cast/nothing/here/"] {
        for headers in [vec![], vec![("x-api-key", "key-admin")]] {
            let r = send(&w.router, "GET", path, &headers);
            assert_eq!((r.status, &r.body), (404, &json!({ "error": "Page not found." })), "{path} {headers:?}");
            assert!(!r.has_rate_headers(), "{path}");
        }
    }
}

/// The router the composition roots mount is the same seal over the crate's own routes. It has none yet, so everything under
/// `/api/v1` is the URLconf's 404; what matters here is that it IS sealed (a body in Plane's shape, not an empty 404).
#[test]
fn the_router_the_roots_mount_is_sealed_too() {
    let router = thrust_api_v1::router().layer(axum::Extension(app()));
    let r = send(&router, "GET", "/api/v1/nope/", &[("x-api-key", "key-admin")]);
    assert_eq!((r.status, &r.body), (404, &json!({ "error": "Page not found." })));
    assert!(!r.has_rate_headers());
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// The canonical lower-case form of any uuid is an id, and no other spelling of it is (unless it is the same text).
    #[test]
    fn prop_a_plane_uuid_is_the_canonical_lower_case_form_only(n in any::<u128>()) {
        let id = Uuid::from_u128(n);
        let canonical = id.hyphenated().to_string();
        prop_assert_eq!(PlaneUuid::try_from(canonical.clone()), Ok(PlaneUuid(id)));
        for other in [canonical.to_uppercase(), id.simple().to_string(), id.braced().to_string(), id.urn().to_string()] {
            prop_assert_eq!(PlaneUuid::try_from(other.clone()).is_ok(), other == canonical, "{}", other);
        }
    }
}

// --- the errors ---------------------------------------------------------------------------------------------------------

fn render(e: V1Error) -> (u16, Value, axum::response::Response) {
    let resp = e.into_response();
    let status = resp.status().as_u16();
    let (parts, body) = resp.into_parts();
    let bytes = futures_executor::block_on(axum::body::to_bytes(body, usize::MAX)).unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json, axum::response::Response::from_parts(parts, axum::body::Body::empty()))
}

/// Every error core can raise has an answer, and the answer is Plane's. (The match has no wildcard, so a new `CoreError` is a
/// compile error here until it is given a row.)
#[test]
fn every_core_error_has_a_plane_answer() {
    let cases: Vec<(CoreError, u16, Value)> = vec![
        (CoreError::Unauthenticated, 401, json!({ "detail": DETAIL_NO_KEY })),
        (CoreError::Forbidden, 403, json!({ "detail": DETAIL_DENIED })),
        (CoreError::Csrf, 403, json!({ "detail": DETAIL_DENIED })),
        (CoreError::InvalidApiKey, 403, json!({ "detail": DETAIL_BAD_KEY })),
        (CoreError::NotFound, 404, json!({ "error": "The requested resource does not exist." })),
        (CoreError::Conflict("taken".into()), 409, json!({ "error": "taken" })),
        (
            CoreError::RateLimited { retry_after_secs: 7 },
            429,
            json!({ "error_code": 5900, "error_message": "RATE_LIMIT_EXCEEDED" }),
        ),
        (CoreError::Invalid("bad".into()), 400, json!({ "detail": "bad" })),
        (CoreError::TooLarge, 413, json!({ "detail": "Request entity too large." })),
        (CoreError::Upstream("idp".into()), 500, json!({ "error": "Something went wrong please try again later" })),
        (CoreError::Internal("db".into()), 500, json!({ "error": "Something went wrong please try again later" })),
    ];
    for (e, status, body) in cases {
        match &e {
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
        let label = e.to_string();
        let (got_status, got_body, resp) = render(e.into());
        assert_eq!((got_status, &got_body), (status, &body), "{label}");
        if status == 429 {
            assert_eq!(resp.headers().get("retry-after").and_then(|v| v.to_str().ok()), Some("7"));
        }
    }
}

/// The pin's 500 for a request it fails on purpose carries no cause; one that should not have happened carries the cause for the
/// log, in a response extension and never in the body.
#[test]
fn a_500_keeps_its_cause_out_of_the_body() {
    let (status, body, resp) = render(thrust_api_v1::plane_500());
    assert_eq!((status, &body), (500, &json!({ "error": thrust_api_v1::PLANE_500_MESSAGE })));
    assert!(resp.extensions().get::<thrust_api::InternalErrorDetail>().is_none());

    let (status, body, resp) = render(V1Error::internal("connection reset by peer"));
    assert_eq!((status, &body), (500, &json!({ "error": thrust_api_v1::PLANE_500_MESSAGE })));
    let cause = resp.extensions().get::<thrust_api::InternalErrorDetail>().expect("the cause is attached");
    assert_eq!(cause.0, "connection reset by peer");
}
