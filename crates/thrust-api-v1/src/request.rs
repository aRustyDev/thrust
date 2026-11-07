//! Reading a v1 request the way DRF does: the query string, and a JSON body.

use std::collections::HashMap;

use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use serde_json::{Map, Value};

use crate::V1Error;

/// The query string as `request.GET` has it: a name given twice has its LAST value (`QueryDict.get`), and a name given with no
/// value has the empty one.
pub fn query_map(raw: Option<&str>) -> HashMap<String, String> {
    form_urlencoded::parse(raw.unwrap_or_default().as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

/// A request's JSON body, as DRF reads `request.data`.
#[derive(Debug)]
pub enum Body {
    /// An object, or nothing at all (an empty body is no data).
    Object(Map<String, Value>),
    /// A list: DRF accepts it, and what a route does with it is the route's (the pin's views fail in different ways).
    Array,
}

/// Read the body. An empty body is an empty object; any content type but JSON is a 415 (the pin's other parsers, forms and
/// multipart, are not read here yet); a body that does not parse is a 400 with the parser's message (Python's wording is not
/// reproduced, so the register carries that difference); a JSON value that is neither an object nor a list is a 400.
pub fn json_body(headers: &HeaderMap, bytes: &[u8]) -> Result<Body, V1Error> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Body::Object(Map::new()));
    }
    let sent = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default();
    let essence = sent.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    if essence != "application/json" {
        return Err(V1Error::Detail(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            format!("Unsupported media type \"{sent}\" in request."),
        ));
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => Ok(Body::Object(map)),
        Ok(Value::Array(_)) => Ok(Body::Array),
        Ok(_) => Err(V1Error::Detail(StatusCode::BAD_REQUEST, "Request body must be a JSON object or array.".into())),
        Err(e) => Err(V1Error::Detail(StatusCode::BAD_REQUEST, format!("JSON parse error - {e}"))),
    }
}
