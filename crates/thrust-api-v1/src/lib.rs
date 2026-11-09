//! Plane's public v1 API (`/api/v1`) as a route group over `thrust-core` use-cases (ADR-0031, ADR-0034, issue #58).
//!
//! Every answer here is Plane's, field for field and status for status, because the contract is the oracle
//! (`tools/oracle-v1`: the pinned schema, the exception register and the golden corpus recorded from the pin). A route is a thin
//! reading of the request into a core use-case and of its result into Plane's body; the rules are core's, and the same ones
//! serve the internal API (`thrust-compat-plane`).
//!
//! Plane-shaped types (the bodies, the cursor envelope, the error shapes, `plane_500`) stay in this crate and out of
//! `thrust-contracts` (#52). The crate is runtime-free, as `thrust-api` is: it compiles to wasm32 and runs under workerd.
//!
//! A route takes [`V1Path`] first (a path that does not match is the URLconf's 404, answered before a credential is read), then
//! [`Ctx`] (the API key, the permission, the throttle), and answers `Result<_, V1Error>`.

// `json!` of a project's 43 keys is more than the macro's default depth
#![recursion_limit = "256"]

mod ctx;
mod error;
pub mod fields;
mod labels;
pub mod pagination;
mod projects;
pub mod request;
mod states;
mod work_items;

use axum::routing::any;
use axum::{middleware, Router};

pub use ctx::{Ctx, PlaneUuid, RateInfo, V1Config, V1Path};
pub use error::{plane_500, V1Error, PLANE_500_MESSAGE};
pub use states::slugify;

/// The routes of `/api/v1`, for the composition roots to merge. Each needs the `App` in the request's extensions, as
/// `thrust-api`'s do.
pub fn router() -> Router {
    assemble(projects::routes().merge(states::routes()).merge(labels::routes()).merge(work_items::routes()))
}

/// Finish a set of v1 routes: anything under `/api/v1` that none of them answers is Plane's 404 for an unknown path, and every
/// answer that was counted against a key carries the rate headers. [`router`] is this over every route; a test that mounts a route
/// of its own goes through it too.
pub fn assemble(routes: Router) -> Router {
    routes.route("/api/v1/{*rest}", any(page_not_found)).layer(middleware::from_fn(ctx::rate_headers))
}

async fn page_not_found() -> V1Error {
    V1Error::PageNotFound
}
