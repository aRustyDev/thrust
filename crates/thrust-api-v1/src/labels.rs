//! The labels of Plane's v1 API: list, create, retrieve, change and delete (`/api/v1/workspaces/{slug}/projects/{id}/labels/`).
//!
//! Who may do what is the pin's: a read is for any member of the project, and a write for a project **admin or member**
//! (`ProjectMemberPermission`: a guest is refused, as is a workspace admin who is not in the project). The rules are core's
//! (`thrust_core::labels`); this is the pin's wire shape over them: the 15-key body, DRF's field validation in the order of the
//! serializer's fields, a create that answers 201, and the three different answers to a name that is taken.
//!
//! Where the label views differ from the state views, and this follows the label ones: the queryset of a read, a change and a deletion
//! is the same (live labels, none of an archived project), the detail read ignores `fields`, and a list is ordered by `-created_at`
//! with no `order_by` of its own.

use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use thrust_contracts::{ProjectId, UserId};
use thrust_core::{LabelError, LabelPatch, Permission};
use thrust_ports::LabelRow;

use crate::fields::{
    char_field, fail, float_field, name_was_padded, no_such_pk, no_such_pk_shown, not_a_dictionary, pk_field, read,
    refuse_if_any, FieldErrors, PkRead,
};
use crate::pagination::PageRequest;
use crate::projects::ts;
use crate::request::{json_body, pick_fields, query_map, Body};
use crate::{with_allow, Ctx, PlaneUuid, V1Error, V1Path};

pub(crate) fn routes() -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/labels/",
            with_allow("GET, POST", get(list).post(create)),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/labels/{label_id}/",
            with_allow("GET, PATCH, DELETE", get(retrieve).patch(update).delete(destroy)),
        )
}

// --- the wire shape -----------------------------------------------------------------------------------------------------------

/// The pin's `LabelSerializer` (`fields = "__all__"`): 15 keys.
pub(crate) fn label_body(l: &LabelRow) -> Map<String, Value> {
    let user = |u: Option<UserId>| u.map_or(Value::Null, |u| json!(u.0));
    let body = json!({
        "id": l.id,
        "created_at": ts(l.created_at),
        "updated_at": ts(l.updated_at),
        "deleted_at": null,
        "created_by": user(l.created_by),
        "updated_by": user(l.updated_by),
        "workspace": l.workspace_id.0,
        "project": l.project_id.0,
        "parent": l.parent_id,
        "name": l.name,
        "description": l.description,
        "color": l.color,
        "sort_order": l.sort_order,
        "external_source": l.external_source,
        "external_id": l.external_id,
    });
    match body {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// What the pin answers for a refusal. A name that is taken is a 409 naming the label when a label is made, and the base view's 400
/// "The payload is not valid" when one is changed (the database's integrity error reaches the base view there).
fn label_error(e: LabelError, changing: bool) -> V1Error {
    match e {
        LabelError::NotFound => V1Error::NotFound,
        LabelError::NameTaken { .. } if changing => V1Error::error(StatusCode::BAD_REQUEST, "The payload is not valid"),
        LabelError::NameTaken { id: Some(id) } => V1Error::Error(
            StatusCode::CONFLICT,
            json!({ "error": "Label with the same name already exists in the project", "id": id }),
        ),
        // the clash was with a label that is no longer there to name: the pin's view fails on the missing row
        LabelError::NameTaken { id: None } => {
            V1Error::internal("a label's name is taken by a row that is gone: the pin's view fails on the missing row")
        }
        LabelError::ExternalTaken { id } => V1Error::Error(
            StatusCode::CONFLICT,
            json!({ "error": "Label with the same external id and external source already exists", "id": id }),
        ),
        LabelError::NoSuchParent(parent) => {
            let mut errors = FieldErrors::new();
            fail(&mut errors, "parent", no_such_pk(parent));
            V1Error::Fields(errors)
        }
        LabelError::Core(other) => other.into(),
    }
}

// --- reading a body -----------------------------------------------------------------------------------------------------------

/// What a body says of a label, as the pin's `LabelCreateUpdateSerializer` reads it, field by field in the order the serializer lists
/// them (name, color, description, external_source, external_id, parent, sort_order): what is wrong is collected into one 400, a
/// `parent` the key column cannot read (`“x” is not a valid UUID.`, measured on the pin) included. A field it does not know is ignored.
async fn label_patch(
    ctx: &Ctx,
    scope: &thrust_core::Scope,
    b: &Map<String, Value>,
    partial: bool,
) -> Result<LabelPatch, V1Error> {
    let mut errors = FieldErrors::new();
    if !partial && !b.contains_key("name") {
        fail(&mut errors, "name", "This field is required.");
    }
    let mut patch = LabelPatch {
        name: read(&mut errors, b, "name", |v| char_field(v, Some(255), false, false)).flatten(),
        color: read(&mut errors, b, "color", |v| char_field(v, Some(255), true, false)).flatten(),
        description: read(&mut errors, b, "description", |v| char_field(v, None, true, false)).flatten(),
        external_source: read(&mut errors, b, "external_source", |v| char_field(v, Some(255), true, true)),
        external_id: read(&mut errors, b, "external_id", |v| char_field(v, Some(255), true, true)),
        ..LabelPatch::default()
    };
    if let Some(v) = b.get("parent") {
        match pk_field(v) {
            PkRead::Null => patch.parent = Some(None),
            // the pin's field takes any label of any project; this takes a live one of this project (D-032)
            PkRead::Key { id, shown } => {
                if thrust_core::is_label_of_project(&ctx.app, scope, id).await.map_err(|e| label_error(e, false))? {
                    patch.parent = Some(Some(id));
                } else {
                    fail(&mut errors, "parent", no_such_pk_shown(&shown));
                }
            }
            PkRead::Refused(message) => fail(&mut errors, "parent", message),
        }
    }
    patch.sort_order = read(&mut errors, b, "sort_order", float_field);
    refuse_if_any(errors)?;
    Ok(patch)
}

// --- the routes ---------------------------------------------------------------------------------------------------------------

/// `GET .../labels/`: the project's labels, newest first, paged. There is no `order_by` here (the pin's view orders by the URL's
/// keyword argument, which no route has), and `expand` is ignored (D-032).
async fn list(
    V1Path((slug, project)): V1Path<(String, PlaneUuid)>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::ReadProject).await?;
    let q = query_map(raw.as_deref());
    let plan =
        PageRequest::parse(q.get("per_page").map(String::as_str), q.get("cursor").map(String::as_str))?.plan()?;
    let all = thrust_core::visible_labels(&ctx.app, &scope).await.map_err(|e| label_error(e, false))?;
    let total = all.len();
    let results = plan.slice(all).iter().map(|l| pick_fields(label_body(l), &q)).collect();
    Ok(Json(plan.envelope(results, total)))
}

/// `POST .../labels/`: make a label. The pin answers 201.
async fn create(
    V1Path((slug, project)): V1Path<(String, PlaneUuid)>,
    ctx: Ctx,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::WriteLabel).await?;
    let map = match json_body(&headers, &body)? {
        Body::Object(map) => map,
        Body::Array => return Err(not_a_dictionary()),
    };
    let patch = label_patch(&ctx, &scope, &map, false).await?;
    let padded = name_was_padded(&map, patch.name.as_deref());
    match thrust_core::create_label(&ctx.app, &scope, patch).await {
        // the pin looks the label that has the name up by the text as it was sent, which DRF trimmed before the database saw it: a
        // name sent with spaces round it finds nothing, and the view's `label.id` raises
        Err(LabelError::NameTaken { .. }) if padded => Err(V1Error::internal(
            "a padded label name clashes: the pin looks it up untrimmed, finds nothing, and `label.id` raises",
        )),
        made => {
            let label = made.map_err(|e| label_error(e, false))?;
            Ok((StatusCode::CREATED, Json(Value::Object(label_body(&label)))))
        }
    }
}

/// `GET .../labels/{id}/`. The pin's detail read builds its serializer with neither `fields` nor `expand`: both are ignored.
async fn retrieve(
    V1Path((slug, project, label)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::ReadProject).await?;
    let label = thrust_core::visible_label(&ctx.app, &scope, label.0).await.map_err(|e| label_error(e, false))?;
    Ok(Json(Value::Object(label_body(&label))))
}

/// `PATCH .../labels/{id}/`: change a label. The base view reads the body before the handler runs, so a body that cannot be parsed
/// is refused before the label is looked for; the handler then finds the label (an archived project has none to find), and only then
/// does the serializer read what the body says, so a list, or a field that is wrong, comes after the 404.
async fn update(
    V1Path((slug, project, label)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::WriteLabel).await?;
    let parsed = json_body(&headers, &body)?;
    thrust_core::visible_label(&ctx.app, &scope, label.0).await.map_err(|e| label_error(e, true))?;
    let map = match parsed {
        Body::Object(map) => map,
        Body::Array => return Err(not_a_dictionary()),
    };
    let patch = label_patch(&ctx, &scope, &map, true).await?;
    let label = thrust_core::change_label(&ctx.app, &scope, label.0, patch).await.map_err(|e| label_error(e, true))?;
    Ok(Json(Value::Object(label_body(&label))))
}

/// `DELETE .../labels/{id}/`.
async fn destroy(
    V1Path((slug, project, label)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
) -> Result<StatusCode, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::WriteLabel).await?;
    thrust_core::delete_label(&ctx.app, &scope, label.0).await.map_err(|e| label_error(e, true))?;
    Ok(StatusCode::NO_CONTENT)
}
