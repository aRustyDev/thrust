//! Assets: what the stock app's project creation uploads a cover with (S28, S29, S31, S33), and the three routes around them
//! that only Thrust has (ADR-0047, issue #44).
//!
//! On the pin the browser uploads straight to the object store, to an address S28 presigns. Thrust has no store the browser can
//! reach on every target, so S28 hands it an address of this API instead, with the same `upload_data` the app already reads for
//! Cloudflare R2 (`method: "PUT"`, the headers it signed): `PUT /api/assets/v2/upload/{asset}/{token}/`. The browser sends that
//! upload with no cookies (`withCredentials: false`), so the token in the address is the credential: one use, an hour (D-035).
//!
//! S33 answers 302 as the pin does, to `.../object/`, a route of the same workspace that checks the caller again and sends the
//! bytes (the browser follows a redirect with its cookies, as the pin's presigned URL needed none). A project's cover is shown at
//! `/api/assets/v2/static/{asset}/`, which needs no session, as on the pin.

use std::fmt::Write as _;

use axum::body::{to_bytes, Body, Bytes};
use axum::extract::Path;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};
use thrust_contracts::*;
use thrust_core::{App, AssetError, CoreError, NewUpload, Permission, IMAGE_TYPES};
use thrust_ports::*;

use crate::projects::missing_object;
use crate::users::{object_body, Caller};
use crate::workspace_load::{decorated, refused_by_decorator, ALL_ROLES};
use crate::PlaneError;

/// The ten values of the pin's `FileAsset.EntityTypeContext`; one that is not among them is a 400.
const ENTITY_TYPES: [&str; 10] = [
    "ISSUE_ATTACHMENT",
    "ISSUE_DESCRIPTION",
    "COMMENT_DESCRIPTION",
    "PAGE_DESCRIPTION",
    "USER_COVER",
    "USER_AVATAR",
    "WORKSPACE_LOGO",
    "PROJECT_COVER",
    "DRAFT_ISSUE_ATTACHMENT",
    "DRAFT_ISSUE_DESCRIPTION",
];

/// The address a project's cover is shown at: the pin's `FileAsset.asset_url` for a cover.
pub(crate) fn static_url(id: impl std::fmt::Display) -> String {
    format!("/api/assets/v2/static/{id}/")
}

/// A route's asset id, or the 404 the pin gives a path no route matches: its URLs take `<uuid:asset_id>`, and the fixtures show the
/// asset routes answering `{"error": "Page not found."}` (the project routes' `{"detail": "Not found."}` is another handler's).
fn asset_id(raw: &str) -> Result<AssetId, PlaneError> {
    raw.parse().map_err(|_| PlaneError::NotFound(json!({"error": "Page not found."})))
}

/// Python's truthiness of a JSON value, which is how the pin tells "no ids" from ids.
fn falsy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::Bool(true)) => false,
    }
}

/// What each way an asset's routes refuse is on the pin, and where it is Thrust's own, the words Thrust uses.
fn refusal(e: AssetError) -> PlaneError {
    match e {
        // `FileAsset.objects.get` finds none
        AssetError::NotFound => missing_object(),
        AssetError::NoAccess => PlaneError::Forbidden(json!({"error": "You don't have access to this asset."})),
        AssetError::NotUploaded => PlaneError::NotFound(json!({"error": "The requested asset could not be found."})),
        AssetError::BadType => PlaneError::BadRequest(json!({
            "error": "Invalid file type. Only JPEG, PNG, WebP, JPG and GIF files are allowed.",
            "status": false,
        })),
        AssetError::BadSize => PlaneError::BadRequest(json!({"error": "Invalid file size.", "status": false})),
        AssetError::BadName => PlaneError::BadRequest(json!({"error": "The file name is too long.", "status": false})),
        AssetError::NoIds => PlaneError::BadRequest(json!({"error": "No asset ids provided."})),
        AssetError::NoGrant => {
            PlaneError::Forbidden(json!({"error": "The upload address is not valid, or it has expired or been used."}))
        }
        AssetError::UploadMismatch => {
            PlaneError::Forbidden(json!({"error": "The upload is not the file the address was made for."}))
        }
        AssetError::ObjectMissing => PlaneError::BadRequest(json!({"error": "The file has not been uploaded."})),
        AssetError::ObjectMismatch => {
            PlaneError::BadRequest(json!({"error": "The uploaded file is not the one the asset was made for."}))
        }
        AssetError::Core(e) => e.into(),
    }
}

/// Python's `urllib.parse.quote`, which the pin builds a download's `filename*` with: every byte but a letter, a digit and
/// `_ . - ~ /` as `%XX`.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            // writing to a `String` cannot fail
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The `size` of S28's body as the pin's `int(...)` reads it: a number (cut to a whole one) or a string of digits. Anything else,
/// where the pin's `int` raises and the view answers 500, is no size.
fn size_field(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// The `upload_data` the app reads: the pin's presigned PUT for R2 (`generate_presigned_put`), with the address Thrust's own. The
/// headers are what the upload must send, and the app drops `Content-Length` (a browser sets it from the body).
fn upload_data(asset: &AssetRow, token: &str) -> Value {
    json!({
        "method": "PUT",
        "url": format!("/api/assets/v2/upload/{}/{token}/", asset.id),
        "fields": {},
        "headers": {"Content-Type": asset.content_type, "Content-Length": asset.size.to_string()},
    })
}

/// S28. Make an asset and say where to send its bytes. Any member of the workspace may; a workspace logo needs an admin.
pub(crate) async fn create(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path(slug): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, PlaneError> {
    let scope = decorated(&app, &who, slug, &ALL_ROLES).await?;
    let b = object_body(&body)?;
    let Some(entity_type) = b.get("entity_type").and_then(Value::as_str).filter(|t| ENTITY_TYPES.contains(t)) else {
        return Err(PlaneError::BadRequest(json!({"error": "Invalid entity type.", "status": false})));
    };
    if entity_type == "WORKSPACE_LOGO" && scope.role() != Role::Admin {
        return Err(PlaneError::Forbidden(json!({"error": "Only workspace admins can upload a workspace logo."})));
    }
    let content_type = match b.get("type") {
        None => "image/jpeg",
        Some(Value::String(t)) => t.as_str(),
        Some(_) => "",
    };
    if !IMAGE_TYPES.contains(&content_type) {
        return Err(refusal(AssetError::BadType));
    }
    // `entity_identifier` is the project a cover is for. The pin's default for a key that is not there is `False`, which its
    // database refuses, and a text that is no id fails its validation: both are 400s (fixtures S28), whatever the kind. None is
    // null or the empty text the app sends for a project that does not exist yet.
    let names_a_project = match b.get("entity_identifier") {
        None | Some(Value::Bool(false)) => {
            return Err(PlaneError::BadRequest(json!({"error": "The payload is not valid"})))
        }
        Some(Value::Null) => false,
        Some(Value::String(s)) if s.is_empty() => false,
        Some(Value::String(s)) if s.parse::<Uuid>().is_ok() => true,
        Some(_) => return Err(PlaneError::BadRequest(json!({"error": "Please provide valid detail"}))),
    };
    // Only a project's cover is built (D-035); the pin makes the others, and nothing here would read them
    let Some(entity) = AssetEntity::parse(entity_type) else {
        return Err(PlaneError::BadRequest(json!({
            "error": "This kind of file cannot be uploaded yet.",
            "status": false,
        })));
    };
    // The pin attaches the cover of the project an identifier names when the upload is confirmed. Thrust attaches a cover with S31
    // (or S32's `cover_image_asset`), which is what the project's creation does; changing an existing project's cover through
    // here is refused loudly rather than accepted and not applied (D-035).
    if names_a_project {
        return Err(PlaneError::BadRequest(json!({
            "error": "The cover of an existing project cannot be changed this way yet.",
            "status": false,
        })));
    }
    let size = match b.get("size") {
        None => None,
        Some(v) => Some(size_field(v).ok_or_else(|| refusal(AssetError::BadSize))?),
    };
    let name = b.get("name").and_then(Value::as_str).map(str::to_string);
    let ticket = thrust_core::create_asset(
        &app,
        &scope,
        NewUpload { name, content_type: content_type.to_string(), size, entity },
    )
    .await
    .map_err(refusal)?;
    Ok(Json(json!({
        "upload_data": upload_data(&ticket.asset, &ticket.token),
        "asset_id": ticket.asset.id.0,
        "asset_url": static_url(ticket.asset.id),
    })))
}

/// The browser's upload: the bytes of the file, with the type the asset was made for, to the address S28 gave. No session, so the
/// token is the credential; a body longer than the limit is refused before the token is touched.
pub(crate) async fn upload(
    Extension(app): Extension<App>,
    Path((asset, token)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, PlaneError> {
    let id = asset_id(&asset)?;
    let bytes = to_bytes(body, app.cfg.max_attachment_bytes)
        .await
        .map_err(|_| PlaneError::Other(CoreError::TooLarge))?
        .to_vec();
    let content_type = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok());
    thrust_core::receive_upload(&app, id, &token, content_type, bytes).await.map_err(refusal)?;
    Ok(StatusCode::OK)
}

/// S29. The browser says the upload is done: 204. The pin takes its word; this looks at the object first (D-035).
pub(crate) async fn confirm(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, asset)): Path<(String, String)>,
) -> Result<StatusCode, PlaneError> {
    let scope = decorated(&app, &who, slug, &ALL_ROLES).await?;
    thrust_core::confirm_upload(&app, &scope, asset_id(&asset)?).await.map_err(refusal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// S31. Attach the caller's uploads to a project, which is how a cover uploaded before its project existed becomes its cover: 204.
/// What the caller did not upload, or another project has, is left out without a word; if nothing is left it is a 404.
pub(crate) async fn bulk(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project, _entity)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<StatusCode, PlaneError> {
    let project = crate::projects::project_id(&project)?;
    let b = object_body(&body)?;
    let slug = WorkspaceSlug::new(slug);
    // the decorator answers a stranger and a non-member alike
    let by_decorator = |e: CoreError| match e {
        CoreError::NotFound | CoreError::Forbidden => refused_by_decorator(),
        other => other.into(),
    };
    // What the pin does with `asset_ids`, after its decorator: nothing that is truthy-empty (null, `[]`, `""`, none) is "No asset
    // ids provided", and a value that is not a list of ids (a text, or a list with a text that is no id) fails the query's own
    // validation, `Please provide valid detail`. Ids that no asset has are left out without a word.
    let asked = b.get("asset_ids");
    let ids: Option<Vec<AssetId>> = if falsy(asked) {
        Some(vec![])
    } else {
        asked
            .and_then(Value::as_array)
            .and_then(|a| a.iter().map(|v| v.as_str().and_then(|s| s.parse().ok())).collect::<Option<Vec<AssetId>>>())
    };
    let Some(ids) = ids else {
        thrust_core::authorize(&app, &who, &slug, project, Permission::ReadProject).await.map_err(by_decorator)?;
        return Err(PlaneError::BadRequest(json!({"error": "Please provide valid detail"})));
    };
    thrust_core::attach_project_cover(&app, &who, &slug, project, &ids).await.map_err(|e| match e {
        AssetError::NotFound => PlaneError::NotFound(json!({"error": "The requested asset could not be found."})),
        AssetError::Core(e) => by_decorator(e),
        other => refusal(other),
    })?;
    Ok(StatusCode::NO_CONTENT)
}

/// S33. Where to download an asset from: 302 to the route that sends it. The pin presigns a URL of the object store; the caller's
/// cookies go with a redirect to the same origin, so the target asks who they are again.
pub(crate) async fn redirect(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, asset)): Path<(String, String)>,
) -> Result<Response, PlaneError> {
    let scope = decorated(&app, &who, slug.clone(), &ALL_ROLES).await?;
    let asset = thrust_core::open_asset(&app, &scope, asset_id(&asset)?).await.map_err(refusal)?;
    let location = format!("/api/assets/v2/workspaces/{slug}/{}/object/", asset.id);
    // the redirect Django's `HttpResponseRedirect` makes, with the type it gives it (fixture S33)
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, location), (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
    )
        .into_response())
}

/// What S33 sends the browser to: the asset's bytes, to be saved (`attachment`, with the file's name), for whoever may read it.
pub(crate) async fn object(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, asset)): Path<(String, String)>,
) -> Result<Response, PlaneError> {
    let scope = decorated(&app, &who, slug, &ALL_ROLES).await?;
    let asset = thrust_core::open_asset(&app, &scope, asset_id(&asset)?).await.map_err(refusal)?;
    let blob = thrust_core::read_asset(&app, &asset).await.map_err(refusal)?;
    file_response(&asset, blob, "attachment", "private, no-store")
}

/// The address a project's cover is shown at, for anyone: the picture itself. The pin redirects to a presigned URL; every kind of
/// asset this route serves is an allow-listed image, so it is shown inline, with its type pinned and sniffing off.
pub(crate) async fn static_asset(
    Extension(app): Extension<App>,
    Path(asset): Path<String>,
) -> Result<Response, PlaneError> {
    let asset = thrust_core::public_asset(&app, asset_id(&asset)?).await.map_err(refusal)?;
    let blob = thrust_core::read_asset(&app, &asset).await.map_err(refusal)?;
    file_response(&asset, blob, "inline", "public, max-age=3600")
}

fn file_response(asset: &AssetRow, blob: Blob, disposition: &str, cache: &'static str) -> Result<Response, PlaneError> {
    let invalid = |_| PlaneError::Other(CoreError::Internal("an asset's headers are not valid".into()));
    let mut resp = Response::new(Body::from(blob.bytes));
    let h = resp.headers_mut();
    // the type was checked against the allow-list when the asset was made
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(&asset.content_type).map_err(invalid)?);
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("{disposition}; filename*=UTF-8''{}", quote(&asset.name))).map_err(invalid)?,
    );
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    // (b) what Python's `quote` makes of a few names, the pin's `_get_content_disposition`
    #[rstest::rstest]
    #[case("cover.png", "cover.png")]
    #[case("my cover.png", "my%20cover.png")]
    #[case("naïve.png", "na%C3%AFve.png")]
    #[case("a+b&c=d.png", "a%2Bb%26c%3Dd.png")]
    #[case("a/b", "a/b")]
    #[case("100%.png", "100%25.png")]
    #[case("~tilde_under-dash.gif", "~tilde_under-dash.gif")]
    fn quote_cases(#[case] name: &str, #[case] quoted: &str) {
        assert_eq!(quote(name), quoted);
    }

    #[rstest::rstest]
    #[case(json!(1234), Some(1234))]
    #[case(json!(12.9), Some(12))]
    #[case(json!("1234"), Some(1234))]
    #[case(json!(" 77 "), Some(77))]
    #[case(json!("big"), None)]
    #[case(json!(null), None)]
    #[case(json!([1]), None)]
    #[case(json!(true), None)]
    fn size_field_cases(#[case] v: Value, #[case] size: Option<i64>) {
        assert_eq!(size_field(&v), size);
    }
}
