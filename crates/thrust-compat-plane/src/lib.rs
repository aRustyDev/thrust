//! Plane's internal API (`/api/...`, `/auth/...`) as one route group (ADR-0034, issue #44).
//!
//! The stock Plane React app calls these routes; each one is a thin adapter from Plane's wire shape to a
//! `thrust-core` use-case. Plane-shaped request and response types stay in this crate (issue #52, FR-20): `thrust-core`,
//! `thrust-ports` and `thrust-contracts` never see them, and `scripts/check-deps.sh` keeps this crate runtime-free so it
//! builds for wasm32.
//!
//! The composition roots mount it next to `thrust_api::router()`:
//!
//! ```ignore
//! thrust_api::router().merge(thrust_compat_plane::routes())
//! ```
//!
//! and inject the same `App` and `ApiConfig` extensions the shared router reads. `/api/v1` (wire-compatible routes,
//! issue #47) and `/api/_slice` belong to `thrust-api` and are not touched here.
//!
//! Every row has a `parity_` test (`tests/parity.rs`) that replays a fixture recorded from the pinned Plane
//! (`oracle/v1-pin-20261018`, `ba85d4d3d3`) with `tools/compat-record/record.py`.

use axum::routing::get;
use axum::Router;

mod error;
mod instance;
mod users;

pub use error::PlaneError;

/// The route group. Row ids (`S01`...) are those of `research/0003` section 8.1.
pub fn routes() -> Router {
    Router::new()
        .route("/api/instances/", get(instance::get_instance)) // S01
        .route("/api/users/me/", get(users::get_me)) // S02
}
