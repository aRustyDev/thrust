//! The pin's `external` views (`apps/api/plane/app/views/external/base.py`): what the app asks a third party for through the API.
//!
//! X08, the Unsplash pictures the project form's cover picker offers, is the one the first journey calls. It is a stub (D-021): the
//! pin proxies api.unsplash.com when an `UNSPLASH_ACCESS_KEY` is configured, and answers an empty list when it is not. Thrust has no
//! such setting and calls no third party, so it answers as an unconfigured pin does.

use axum::Json;
use serde_json::{json, Value};

use crate::users::Caller;

/// X08. The pictures the cover picker lists, for any signed-in caller: none, whatever `query`, `page` and `per_page` say. A caller
/// with no session is refused by [`Caller`] with the pin's 401.
pub(crate) async fn unsplash(Caller(_who): Caller) -> Json<Value> {
    Json(json!([]))
}
