//! Plane's error bodies. A body is added here only when a fixture recorded from the pin shows its shape; until then a
//! failure keeps the shape `thrust-api` gives it.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;
use thrust_core::CoreError;

pub enum PlaneError {
    /// No session, or an expired one. The React client sends every 401 to the sign-in page, so this status means only
    /// that (research/0003 section 9, row 2); a role failure is a 403.
    Unauthenticated,
    /// The route exists but the data behind it does not yet (issue #44 builds the slice stage by stage). A 501, never a
    /// half-shaped 200 that the React app would render.
    NotYet(&'static str),
    /// Anything whose Plane shape has not been recorded yet.
    Other(CoreError),
}

impl From<CoreError> for PlaneError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Unauthenticated => PlaneError::Unauthenticated,
            other => PlaneError::Other(other),
        }
    }
}

impl From<thrust_api::ApiError> for PlaneError {
    fn from(e: thrust_api::ApiError) -> Self {
        e.0.into()
    }
}

impl IntoResponse for PlaneError {
    fn into_response(self) -> Response<Body> {
        match self {
            // The body DRF gives (fixture S02).
            PlaneError::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                Json(json!({"detail": "Authentication credentials were not provided."})),
            )
                .into_response(),
            PlaneError::NotYet(what) => (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": what}))).into_response(),
            PlaneError::Other(e) => thrust_api::ApiError(e).into_response(),
        }
    }
}
