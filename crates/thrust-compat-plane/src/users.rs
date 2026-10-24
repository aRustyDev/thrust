//! `/api/users/me/` and the rows beside it.

use thrust_api::{ApiError, Authed};

use crate::PlaneError;

/// S02. Signed out, the pin answers 401 with `{"detail": ...}`: a JSON object with no `id`, which is how the React app
/// learns it is signed out (research/0003 section 9, row 3). The signed-in answer (the user, with the fields the app
/// reads) needs the profile tables of issue #44's boot and onboarding stage, so until then it is a 501.
pub(crate) async fn get_me(who: Result<Authed, ApiError>) -> PlaneError {
    match who {
        Err(e) => e.into(),
        Ok(_) => PlaneError::NotYet("GET /api/users/me/ for a signed-in user needs the profile tables (issue #44, boot stage)"),
    }
}
