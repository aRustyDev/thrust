//! The 500 answers of the pinned Plane (`oracle/v1-pin-20261018`, `ba85d4d3d3`), and the one function that gives them (#51).
//!
//! The owner decided in round 27 to reproduce every Plane 500 as the same status and body, returned as a handled error and
//! never as a crash of the process (ADR-0031, ADR-0006). The text of a 500 is part of the contract (`research/0007` PQ-409),
//! and the pin has three. Both Plane-shaped surfaces, `/api/v1` and the internal API of `thrust-compat-plane`, answer a
//! reproduced 500 through [`plane_500`], so no handler writes `StatusCode::INTERNAL_SERVER_ERROR` or a body of its own
//! (`scripts/check-plane-500.sh` holds the two crates to that).
//!
//! The pin logs the exception (`plane.exception`) and never returns it. [`plane_500`] does the same: the cause rides on the
//! response as [`InternalErrorDetail`], which the composition root logs, and is never in the body.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;
use thrust_api::InternalErrorDetail;

/// The base handlers' catch-all, v1 and internal (`apps/api/plane/api/views/base.py:107`, `apps/api/plane/app/views/base.py:107`).
/// Recorded on the pin: `tools/oracle-v1/corpus/assets.json#workspace-asset-upload` and `pagination.json#per-page-zero`.
pub const SOMETHING_WENT_WRONG: &str = "Something went wrong please try again later";
/// Project creation's own catch-all, on the v1 route only (`apps/api/plane/api/views/project.py:320,337`). Source only: the
/// harness never reached it.
pub const UNEXPECTED_ERROR: &str = "An unexpected error occurred";
/// The generic asset GET's blanket `except` (`apps/api/plane/api/views/asset.py:474`). Source only: the harness never reached it.
pub const INTERNAL_SERVER_ERROR_TEXT: &str = "Internal server error";

/// Which of the pin's three 500 texts an answer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane500 {
    /// `{"error":"Something went wrong please try again later"}`: any exception the base handlers do not map.
    SomethingWentWrong,
    /// `{"error":"An unexpected error occurred"}`: project create's catch-all.
    UnexpectedError,
    /// `{"error":"Internal server error"}`: the generic asset GET's catch-all.
    InternalServerError,
}

impl Plane500 {
    /// The text of the `error` key, as the pin spells it.
    pub const fn message(self) -> &'static str {
        match self {
            Plane500::SomethingWentWrong => SOMETHING_WENT_WRONG,
            Plane500::UnexpectedError => UNEXPECTED_ERROR,
            Plane500::InternalServerError => INTERNAL_SERVER_ERROR_TEXT,
        }
    }
}

/// A reproduced 500: status 500 and `{"error": <the kind's text>}`, with `cause` attached as [`InternalErrorDetail`] so the
/// composition root can log why. A request the pin fails on purpose (a `per_page` of 0, a `deleted_at` in a work item's body)
/// passes a short cause naming the rule; one that should not have happened passes what went wrong. Either way it stays out of
/// the body.
pub fn plane_500(kind: Plane500, cause: impl Into<String>) -> Response<Body> {
    let mut resp = (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": kind.message() }))).into_response();
    resp.extensions_mut().insert(InternalErrorDetail(cause.into()));
    resp
}
