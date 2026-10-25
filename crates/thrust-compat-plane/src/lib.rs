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

use axum::routing::{get, patch, post};
use axum::Router;

mod auth;
mod error;
mod fields;
#[cfg(test)]
mod fields_tests;
mod instance;
mod users;
mod workspaces;

pub use error::PlaneError;

/// The route group. Row ids (`S01`...) are those of `research/0003` section 8.1; `X04` is a route the stock onboarding
/// calls that the inventory does not list.
pub fn routes() -> Router {
    Router::new()
        .route("/api/instances/", get(instance::get_instance)) // S01
        .route("/api/users/me/", get(users::get_me).patch(users::patch_me)) // S02, S10
        .route("/api/users/me/profile/", get(users::get_profile).patch(users::patch_profile)) // S07, S11
        .route("/api/users/me/tour-completed/", patch(users::patch_tour_completed)) // S27
        .route("/api/users/me/onboard/", patch(users::patch_onboard)) // X04
        .route("/api/users/me/settings/", get(workspaces::settings)) // S08
        .route("/api/users/me/workspaces/", get(workspaces::my_workspaces)) // S09
        .route("/api/users/me/workspaces/invitations/", get(workspaces::invitations)) // S12
        .route("/api/workspace-slug-check/", get(workspaces::slug_check)) // S13
        .route("/api/workspaces/", post(workspaces::create)) // S14
        .route("/api/workspaces/{slug}/workspace-members/me/", get(workspaces::member_me)) // S15
        .route("/api/users/me/workspaces/{slug}/project-roles/", get(workspaces::project_roles)) // S16
        .route("/api/workspaces/{slug}/members/", get(workspaces::members)) // S18
        .route("/auth/get-csrf-token/", get(auth::get_csrf_token)) // S03
        .route("/auth/email-check/", post(auth::email_check)) // S04
        .route("/auth/sign-up/", post(auth::sign_up)) // S05
        .route("/auth/sign-in/", post(auth::sign_in)) // S06
}
