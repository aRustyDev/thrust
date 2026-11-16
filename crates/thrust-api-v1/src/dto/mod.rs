//! The bodies the v1 routes answer with, as types (issue #58, ADR-0032).
//!
//! A route builds its body from one of these and serializes it, so the shape on the wire and the shape the OpenAPI document
//! describes are one definition: with the `openapi` feature each type derives `ToSchema`, and the document is written from them.
//! A body is still read from a request by hand (`fields.rs`), because that reading is DRF's (its coercions and its messages); the
//! request types the document needs are in `openapi.rs`, and they are descriptions, not what a route reads.
//!
//! One module per resource, as the routes are: a resource's bodies move here when its routes are described.

mod labels;
mod projects;
mod states;

pub(crate) use labels::LabelDto;
pub(crate) use projects::{ProjectDto, ProjectLiteDto};
pub(crate) use states::StateDto;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::V1Error;

/// A body as the JSON object it serializes to, for a route that then keeps some of its keys (`fields=`, [`crate::request::pick_fields`]).
///
/// A type whose serialization is not an object is a defect of the type, answered as the pin's handled 500 and not as an empty body.
pub(crate) fn into_object(body: &impl Serialize) -> Result<Map<String, Value>, V1Error> {
    match serde_json::to_value(body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(V1Error::internal("a body that is not a JSON object")),
        Err(e) => Err(V1Error::internal(format!("a body that does not serialize: {e}"))),
    }
}
