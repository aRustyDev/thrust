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

// a project's answer is one `json!` of 44 keys, which is more than the macro's default depth allows
#![recursion_limit = "256"]

use axum::routing::{get, patch, post};
use axum::Router;

mod auth;
mod error;
mod fields;
#[cfg(test)]
mod fields_tests;
mod instance;
mod projects;
#[cfg(test)]
mod projects_tests;
mod users;
mod workspace_load;
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
        .route("/api/workspaces/{slug}/projects/", get(workspace_load::projects).post(projects::create)) // S17, S30
        .route("/api/workspaces/{slug}/projects/{project}/", get(projects::retrieve).patch(projects::update)) // S34, S32
        .route("/api/workspaces/{slug}/project-stats/", get(projects::stats)) // S35
        .route("/api/workspaces/{slug}/projects/{project}/project-members/me/", get(projects::member_me)) // S36
        .route("/api/workspaces/{slug}/projects/{project}/user-properties/", get(projects::user_properties)) // S37
        .route("/api/workspaces/{slug}/projects/{project}/issue-labels/", get(projects::labels)) // S38
        .route("/api/workspaces/{slug}/projects/{project}/members/", get(projects::members)) // S39
        .route("/api/workspaces/{slug}/projects/{project}/states/", get(projects::states)) // S40
        .route("/api/workspaces/{slug}/projects/{project}/intake-state/", get(projects::intake_state)) // S41
        .route("/api/workspaces/{slug}/projects/{project}/estimates/", get(projects::estimates)) // S42
        .route("/api/workspaces/{slug}/projects/{project}/cycles/", get(projects::project_stub)) // S43
        .route("/api/workspaces/{slug}/modules/", get(projects::workspace_modules)) // S44
        .route("/api/workspaces/{slug}/projects/{project}/modules/", get(projects::project_stub)) // S45
        .route("/api/workspaces/{slug}/projects/{project}/views/", get(projects::project_stub)) // S46
        .route("/api/workspaces/{slug}/members/", get(workspaces::members)) // S18
        .route("/api/workspaces/{slug}/user-favorites/", get(workspace_load::favorites)) // S19
        .route("/api/workspaces/{slug}/states/", get(workspace_load::states)) // S20
        .route("/api/workspaces/{slug}/sidebar-preferences/", get(workspace_load::sidebar_preferences)) // S21
        .route("/api/workspaces/{slug}/user-properties/", get(workspace_load::user_properties)) // S22
        .route("/api/workspaces/{slug}/users/notifications/unread/", get(workspace_load::unread_notifications)) // S23
        .route("/api/workspaces/{slug}/home-preferences/", get(workspace_load::home_preferences)) // S24
        .route("/api/workspaces/{slug}/quick-links/", get(workspace_load::quick_links)) // S25
        .route("/api/workspaces/{slug}/recent-visits/", get(workspace_load::recent_visits)) // S26
        .route("/auth/get-csrf-token/", get(auth::get_csrf_token)) // S03
        .route("/auth/email-check/", post(auth::email_check)) // S04
        .route("/auth/sign-up/", post(auth::sign_up)) // S05
        .route("/auth/sign-in/", post(auth::sign_in)) // S06
}
